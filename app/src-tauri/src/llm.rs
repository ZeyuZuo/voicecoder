use crate::env_config::read_local_env;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, future::Future, pin::Pin, time::Duration};
use tokio::time::Instant;

const DEFAULT_LLM_TEMPERATURE: f32 = 0.2;
const DEFAULT_LLM_TIMEOUT_SECS: u64 = 30;
const DEFAULT_STRICT_JSON_MODE: bool = true;
const MAX_PROCESSING_QUESTIONS: usize = 3;

type LlmCompletionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<LlmJsonResponse, String>> + Send + 'a>>;

pub(crate) trait LlmProvider {
    fn kind(&self) -> LlmProviderKind;
    fn validate_start(&self) -> Result<(), String>;
    fn diagnostic(&self) -> LlmProviderDiagnostic;
    fn complete_json<'a>(&'a self, request: LlmJsonRequest) -> LlmCompletionFuture<'a>;
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    Auto,
    OpenaiCompatible,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderStatus {
    auto_provider: LlmProviderKind,
    provider_override: Option<LlmProviderKind>,
    active_provider_configured: bool,
    active_provider_error: Option<String>,
    diagnostics: Vec<LlmProviderDiagnostic>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderDiagnostic {
    provider: LlmProviderKind,
    configured: bool,
    missing_env: Vec<String>,
    endpoint: Option<String>,
    model: Option<String>,
    details: BTreeMap<String, String>,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmConnectionTestResult {
    ok: bool,
    provider: LlmProviderKind,
    model: Option<String>,
    endpoint: Option<String>,
    duration_ms: u128,
    response: Option<Value>,
    error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct LlmJsonRequest {
    system_prompt: String,
    user_payload: Value,
    temperature: Option<f32>,
}

#[derive(Clone, Debug)]
pub(crate) struct LlmJsonResponse {
    provider: LlmProviderKind,
    model: String,
    endpoint: String,
    content: Value,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementProcessingRequest {
    state: RequirementStatePayload,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSummaryRequest {
    state: RequirementStatePayload,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementProcessingResult {
    summary: String,
    requirement_document_draft: String,
    confirmed_facts: Vec<String>,
    constraints: Vec<String>,
    acceptance_criteria: Vec<String>,
    out_of_scope: Vec<String>,
    risks: Vec<String>,
    questions: Vec<RequirementQuestionPayload>,
    ready_to_confirm: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSummaryResult {
    summary: String,
    uncertainties: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementStatePayload {
    id: String,
    status: String,
    utterances: Vec<RequirementUtterancePayload>,
    summary: String,
    requirement_document: String,
    confirmed_facts: Vec<String>,
    constraints: Vec<String>,
    open_questions: Vec<RequirementQuestionPayload>,
    answered_questions: Vec<RequirementQuestionPayload>,
    active_question_id: Option<String>,
    acceptance_criteria: Vec<String>,
    out_of_scope: Vec<String>,
    risks: Vec<String>,
    coding_prompt: Option<String>,
    pending_action: Option<String>,
    updated_at: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementUtterancePayload {
    id: String,
    source: String,
    speaker_id: Option<String>,
    text: String,
    created_at: String,
    transcript_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementQuestionPayload {
    id: Option<String>,
    question: String,
    reason: String,
    blocks_coding: bool,
    answer: Option<String>,
}

pub(crate) struct LlmProviderRegistry;

impl LlmProviderRegistry {
    pub(crate) fn provider_for_kind(
        provider: LlmProviderKind,
    ) -> Result<Box<dyn LlmProvider + Send>, String> {
        match provider {
            LlmProviderKind::Auto => {
                Err("auto LLM provider must be resolved before use".to_string())
            }
            LlmProviderKind::OpenaiCompatible => Ok(Box::new(OpenaiCompatibleLlmProvider)),
        }
    }

    pub(crate) fn diagnostics() -> Vec<LlmProviderDiagnostic> {
        vec![OpenaiCompatibleLlmProvider.diagnostic()]
    }

    pub(crate) fn resolve_provider(provider: LlmProviderKind) -> LlmProviderKind {
        match provider {
            LlmProviderKind::Auto => {
                Self::provider_override_from_env().unwrap_or(LlmProviderKind::OpenaiCompatible)
            }
            explicit_provider => explicit_provider,
        }
    }

    pub(crate) fn provider_override_from_env() -> Option<LlmProviderKind> {
        read_local_env("VOICECODER_LLM_PROVIDER")
            .and_then(|value| Self::parse_provider_override(&value))
    }

    fn parse_provider_override(value: &str) -> Option<LlmProviderKind> {
        match value.trim().to_lowercase().as_str() {
            "openai_compatible" => Some(LlmProviderKind::OpenaiCompatible),
            "auto" => None,
            _ => None,
        }
    }
}

pub(crate) struct OpenaiCompatibleLlmProvider;

#[derive(Clone, Debug)]
pub(crate) struct OpenaiCompatibleLlmConfig {
    base_url: String,
    api_key: String,
    model: String,
    temperature: f32,
    timeout: Duration,
    strict_json_mode: bool,
}

impl LlmProvider for OpenaiCompatibleLlmProvider {
    fn kind(&self) -> LlmProviderKind {
        LlmProviderKind::OpenaiCompatible
    }

    fn validate_start(&self) -> Result<(), String> {
        OpenaiCompatibleLlmConfig::from_env().map(|_| ())
    }

    fn diagnostic(&self) -> LlmProviderDiagnostic {
        let missing_env = OpenaiCompatibleLlmConfig::missing_required_env();
        if !missing_env.is_empty() {
            return LlmProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: read_local_env("VOICECODER_LLM_BASE_URL"),
                model: read_local_env("VOICECODER_LLM_MODEL"),
                details: default_openai_compatible_details(),
                error: Some("OpenAI-compatible LLM 配置未完整。".to_string()),
            };
        }

        match OpenaiCompatibleLlmConfig::from_env() {
            Ok(config) => {
                let mut details = BTreeMap::new();
                details.insert(
                    "chatCompletionsEndpoint".to_string(),
                    config.chat_completions_endpoint(),
                );
                details.insert("temperature".to_string(), config.temperature.to_string());
                details.insert(
                    "timeoutSecs".to_string(),
                    config.timeout.as_secs().to_string(),
                );
                details.insert(
                    "strictJsonMode".to_string(),
                    config.strict_json_mode.to_string(),
                );
                details.insert(
                    "apiKeyConfigured".to_string(),
                    (!config.api_key.trim().is_empty()).to_string(),
                );

                LlmProviderDiagnostic {
                    provider: self.kind(),
                    configured: true,
                    missing_env,
                    endpoint: Some(config.base_url),
                    model: Some(config.model),
                    details,
                    error: None,
                }
            }
            Err(error) => LlmProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: read_local_env("VOICECODER_LLM_BASE_URL"),
                model: read_local_env("VOICECODER_LLM_MODEL"),
                details: default_openai_compatible_details(),
                error: Some(error),
            },
        }
    }

    fn complete_json<'a>(&'a self, request: LlmJsonRequest) -> LlmCompletionFuture<'a> {
        Box::pin(async move {
            let config = OpenaiCompatibleLlmConfig::from_env()?;
            complete_openai_compatible_json(config, request).await
        })
    }
}

impl OpenaiCompatibleLlmConfig {
    pub(crate) fn from_env() -> Result<Self, String> {
        let base_url = required_env("VOICECODER_LLM_BASE_URL")?;
        let api_key = required_env("VOICECODER_LLM_API_KEY")?;
        let model = required_env("VOICECODER_LLM_MODEL")?;
        let temperature = parse_f32_env("VOICECODER_LLM_TEMPERATURE", DEFAULT_LLM_TEMPERATURE)?;
        let timeout_secs = parse_u64_env("VOICECODER_LLM_TIMEOUT_SECS", DEFAULT_LLM_TIMEOUT_SECS)?;
        let strict_json_mode =
            optional_bool_env("VOICECODER_LLM_STRICT_JSON_MODE", DEFAULT_STRICT_JSON_MODE);

        if !(0.0..=2.0).contains(&temperature) {
            return Err("VOICECODER_LLM_TEMPERATURE 必须在 0 到 2 之间。".to_string());
        }

        if timeout_secs == 0 {
            return Err("VOICECODER_LLM_TIMEOUT_SECS 必须大于 0。".to_string());
        }

        Ok(Self {
            base_url: normalize_base_url(&base_url)?,
            api_key,
            model,
            temperature,
            timeout: Duration::from_secs(timeout_secs),
            strict_json_mode,
        })
    }

    pub(crate) fn missing_required_env() -> Vec<String> {
        [
            "VOICECODER_LLM_BASE_URL",
            "VOICECODER_LLM_API_KEY",
            "VOICECODER_LLM_MODEL",
        ]
        .into_iter()
        .filter(|key| required_env(key).is_err())
        .map(ToString::to_string)
        .collect()
    }

    fn chat_completions_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

#[tauri::command]
pub fn get_llm_provider_status() -> LlmProviderStatus {
    let provider_override = LlmProviderRegistry::provider_override_from_env();
    let auto_provider = LlmProviderRegistry::resolve_provider(LlmProviderKind::Auto);
    let active_provider_error = LlmProviderRegistry::provider_for_kind(auto_provider)
        .and_then(|provider| provider.validate_start())
        .err();

    LlmProviderStatus {
        auto_provider,
        provider_override,
        active_provider_configured: active_provider_error.is_none(),
        active_provider_error,
        diagnostics: LlmProviderRegistry::diagnostics(),
    }
}

#[tauri::command]
pub async fn test_llm_provider_connection() -> LlmConnectionTestResult {
    let started_at = Instant::now();
    let provider_kind = LlmProviderRegistry::resolve_provider(LlmProviderKind::Auto);
    let provider = match LlmProviderRegistry::provider_for_kind(provider_kind) {
        Ok(provider) => provider,
        Err(error) => {
            return LlmConnectionTestResult {
                ok: false,
                provider: provider_kind,
                model: read_local_env("VOICECODER_LLM_MODEL"),
                endpoint: read_local_env("VOICECODER_LLM_BASE_URL"),
                duration_ms: started_at.elapsed().as_millis(),
                response: None,
                error: Some(error),
            };
        }
    };

    let result = provider
        .complete_json(LlmJsonRequest {
            system_prompt: "Return only a valid JSON object. No prose, no markdown.".to_string(),
            user_payload: json!({
                "task": "health_check",
                "instruction": "Return exactly this semantic result as JSON: {\"ok\": true}.",
                "requiredSchema": {
                    "ok": "boolean true"
                }
            }),
            temperature: Some(0.0),
        })
        .await;

    match result {
        Ok(response) => {
            let ok = response
                .content
                .get("ok")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            LlmConnectionTestResult {
                ok,
                provider: response.provider,
                model: Some(response.model),
                endpoint: Some(response.endpoint),
                duration_ms: started_at.elapsed().as_millis(),
                response: Some(response.content),
                error: if ok {
                    None
                } else {
                    Some("LLM 健康检查响应缺少 ok=true。".to_string())
                },
            }
        }
        Err(error) => LlmConnectionTestResult {
            ok: false,
            provider: provider_kind,
            model: read_local_env("VOICECODER_LLM_MODEL"),
            endpoint: read_local_env("VOICECODER_LLM_BASE_URL"),
            duration_ms: started_at.elapsed().as_millis(),
            response: None,
            error: Some(error),
        },
    }
}

#[tauri::command]
pub async fn summarize_requirement_state(
    request: RequirementSummaryRequest,
) -> Result<RequirementSummaryResult, String> {
    let response = complete_with_active_provider(LlmJsonRequest {
        system_prompt: requirement_summary_system_prompt(),
        user_payload: json!({
            "task": "summarize_requirement_state",
            "state": request.state,
            "outputSchema": {
                "summary": "string, <= 120 Chinese characters when possible",
                "uncertainties": ["string"]
            }
        }),
        temperature: Some(0.1),
    })
    .await?;

    parse_requirement_summary_result(response.content)
}

#[tauri::command]
pub async fn process_requirement_turn(
    request: RequirementProcessingRequest,
) -> Result<RequirementProcessingResult, String> {
    let response = complete_with_active_provider(LlmJsonRequest {
        system_prompt: requirement_processing_system_prompt(),
        user_payload: json!({
            "task": "process_requirement_turn",
            "state": request.state,
            "outputSchema": processing_output_schema()
        }),
        temperature: Some(0.15),
    })
    .await?;

    parse_requirement_processing_result(response.content)
}

async fn complete_with_active_provider(request: LlmJsonRequest) -> Result<LlmJsonResponse, String> {
    let provider_kind = LlmProviderRegistry::resolve_provider(LlmProviderKind::Auto);
    let provider = LlmProviderRegistry::provider_for_kind(provider_kind)?;
    provider.complete_json(request).await
}

async fn complete_openai_compatible_json(
    config: OpenaiCompatibleLlmConfig,
    request: LlmJsonRequest,
) -> Result<LlmJsonResponse, String> {
    let endpoint = config.chat_completions_endpoint();
    let client = reqwest::Client::builder()
        .timeout(config.timeout)
        .build()
        .map_err(|error| format!("LLM HTTP client 初始化失败：{error}"))?;
    let body = build_openai_compatible_request_body(&config, &request);
    let http_response = client
        .post(&endpoint)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("LLM 请求失败：{error}"))?;
    let status = http_response.status();
    let response_text = http_response
        .text()
        .await
        .map_err(|error| format!("读取 LLM 响应失败：{error}"))?;

    if !status.is_success() {
        return Err(format!(
            "LLM HTTP {}：{}",
            status.as_u16(),
            extract_openai_error_message(&response_text)
        ));
    }

    let response_value = serde_json::from_str::<Value>(&response_text)
        .map_err(|error| format!("LLM 响应不是合法 JSON：{error}"))?;
    let content = extract_chat_completion_content(&response_value)?;
    let parsed_content = parse_json_object_content(content)?;

    Ok(LlmJsonResponse {
        provider: LlmProviderKind::OpenaiCompatible,
        model: config.model,
        endpoint,
        content: parsed_content,
    })
}

fn build_openai_compatible_request_body(
    config: &OpenaiCompatibleLlmConfig,
    request: &LlmJsonRequest,
) -> Value {
    let mut body = json!({
        "model": config.model,
        "temperature": request.temperature.unwrap_or(config.temperature),
        "messages": [
            {
                "role": "system",
                "content": request.system_prompt
            },
            {
                "role": "user",
                "content": request.user_payload.to_string()
            }
        ]
    });

    if config.strict_json_mode {
        body["response_format"] = json!({ "type": "json_object" });
    }

    body
}

fn extract_chat_completion_content(response: &Value) -> Result<&str, String> {
    response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "LLM 响应缺少 choices[0].message.content。".to_string())
}

fn parse_json_object_content(content: &str) -> Result<Value, String> {
    let cleaned = strip_markdown_json_fence(content.trim());
    let parsed = serde_json::from_str::<Value>(&cleaned)
        .map_err(|error| format!("LLM 输出不是合法 JSON：{error}"))?;
    if !parsed.is_object() {
        return Err("LLM 输出必须是 JSON object。".to_string());
    }
    Ok(parsed)
}

fn strip_markdown_json_fence(content: &str) -> String {
    let trimmed = content.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }

    let without_opening = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim();
    without_opening
        .strip_suffix("```")
        .unwrap_or(without_opening)
        .trim()
        .to_string()
}

fn extract_openai_error_message(response_text: &str) -> String {
    serde_json::from_str::<Value>(response_text)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message").or(Some(error)))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| response_text.chars().take(600).collect())
}

fn parse_requirement_summary_result(value: Value) -> Result<RequirementSummaryResult, String> {
    let result = serde_json::from_value::<RequirementSummaryResult>(value)
        .map_err(|error| format!("LLM 总结结果结构不正确：{error}"))?;
    if result.summary.trim().is_empty() {
        return Err("LLM 总结结果缺少 summary。".to_string());
    }
    Ok(RequirementSummaryResult {
        summary: limit_text(result.summary, 300),
        uncertainties: result
            .uncertainties
            .into_iter()
            .map(|item| limit_text(item, 160))
            .filter(|item| !item.trim().is_empty())
            .take(5)
            .collect(),
    })
}

fn parse_requirement_processing_result(
    value: Value,
) -> Result<RequirementProcessingResult, String> {
    let mut result = serde_json::from_value::<RequirementProcessingResult>(value)
        .map_err(|error| format!("LLM 需求处理结果结构不正确：{error}"))?;
    result.summary = limit_text(result.summary, 500);
    result.requirement_document_draft = limit_text(result.requirement_document_draft, 6000);
    result.confirmed_facts = clean_string_list(result.confirmed_facts, 12, 240);
    result.constraints = clean_string_list(result.constraints, 12, 240);
    result.acceptance_criteria = clean_string_list(result.acceptance_criteria, 12, 240);
    result.out_of_scope = clean_string_list(result.out_of_scope, 12, 240);
    result.risks = clean_string_list(result.risks, 8, 240);
    result.questions = normalize_processing_questions(result.questions);

    if result.summary.trim().is_empty() {
        return Err("LLM 需求处理结果缺少 summary。".to_string());
    }

    if result.ready_to_confirm
        && result
            .questions
            .iter()
            .any(|question| question.blocks_coding)
    {
        return Err("LLM 不能同时返回 readyToConfirm=true 和阻塞问题。".to_string());
    }

    if !result.ready_to_confirm
        && !result
            .questions
            .iter()
            .any(|question| question.blocks_coding)
    {
        return Err(
            "LLM 判断需求不明确时，必须返回至少一个 blocksCoding=true 的问题。".to_string(),
        );
    }

    Ok(result)
}

fn normalize_processing_questions(
    questions: Vec<RequirementQuestionPayload>,
) -> Vec<RequirementQuestionPayload> {
    questions
        .into_iter()
        .filter(|question| !question.question.trim().is_empty())
        .take(MAX_PROCESSING_QUESTIONS)
        .map(|question| RequirementQuestionPayload {
            id: question.id,
            question: limit_text(question.question, 180),
            reason: limit_text(question.reason, 220),
            blocks_coding: question.blocks_coding,
            answer: question.answer.map(|answer| limit_text(answer, 500)),
        })
        .collect()
}

fn clean_string_list(values: Vec<String>, max_items: usize, max_chars: usize) -> Vec<String> {
    values
        .into_iter()
        .map(|value| limit_text(value, max_chars))
        .filter(|value| !value.trim().is_empty())
        .take(max_items)
        .collect()
}

fn limit_text(value: String, max_chars: usize) -> String {
    value.trim().chars().take(max_chars).collect()
}

fn requirement_summary_system_prompt() -> String {
    [
        "你是 VoiceCoder 的实时需求理解助手。",
        "你的任务是根据语音转写维护一段很短的“当前理解”，用于页面右侧小云朵展示。",
        "只总结已经听到的信息；不要提出正式澄清问题，不要判断需求是否完整，不要推进状态机。",
        "如果语音里有噪声、闲聊、重复或 ASR 错字，只保留和软件需求有关的内容。",
        "输出必须是 JSON object，字段只能是 summary 和 uncertainties。",
    ]
    .join("\n")
}

fn requirement_processing_system_prompt() -> String {
    [
        "你是 VoiceCoder 的需求访谈与需求文档整理助手。",
        "你需要根据完整语音转写、已回答的澄清问题和当前结构化状态，判断需求是否足够进入用户确认。",
        "",
        "核心状态机：collecting -> processing -> clarifying -> processing ... -> ready_to_confirm。",
        "你只能通过 readyToConfirm 和 questions 影响下一步：",
        "- readyToConfirm=true 且 questions 为空或没有 blocksCoding=true 时，前端进入 ready_to_confirm。",
        "- readyToConfirm=false 时，必须返回 1-3 个 blocksCoding=true 的关键问题，前端进入 clarifying。",
        "",
        "避免无限澄清循环的规则：",
        "- 不要重复询问 answeredQuestions 中已经回答过的问题。",
        "- 如果用户已经给出可执行的方向，即使不完美，也应采用合理默认值并进入 readyToConfirm。",
        "- 只问会阻塞实现、验收、范围或关键交互的问题。",
        "- 不要询问颜色、文案、动效、技术实现偏好等 Coding Agent 可以合理判断的细节。",
        "- 每一轮最多问 3 个问题，优先问一个最关键的问题。",
        "- 如果当前问题已经被用户用 clarification_answer 回答，请把回答并入需求，不要继续卡在同一个问题。",
        "",
        "需求明确的最低标准：",
        "- 能说清用户要做什么或改变什么。",
        "- 能说清主要交互或业务流程。",
        "- 能形成至少 1 条可验证的验收标准；如果用户没有明确验收标准，你可以从目标中提炼可验证标准。",
        "- 已知范围和不做范围足以避免明显误实现；不确定但不阻塞的内容写入 risks，不要追问。",
        "",
        "输出必须是严格 JSON object，字段必须符合用户消息里的 outputSchema。不要输出 markdown 或解释。",
    ]
    .join("\n")
}

fn processing_output_schema() -> Value {
    json!({
        "summary": "string",
        "requirementDocumentDraft": "string, user-facing requirement document draft in Chinese",
        "confirmedFacts": ["string"],
        "constraints": ["string"],
        "acceptanceCriteria": ["string"],
        "outOfScope": ["string"],
        "risks": ["string"],
        "questions": [
            {
                "question": "string",
                "reason": "string",
                "blocksCoding": "boolean",
                "answer": "string optional"
            }
        ],
        "readyToConfirm": "boolean"
    })
}

fn default_openai_compatible_details() -> BTreeMap<String, String> {
    let mut details = BTreeMap::new();
    details.insert(
        "temperature".to_string(),
        read_local_env("VOICECODER_LLM_TEMPERATURE")
            .unwrap_or_else(|| DEFAULT_LLM_TEMPERATURE.to_string()),
    );
    details.insert(
        "timeoutSecs".to_string(),
        read_local_env("VOICECODER_LLM_TIMEOUT_SECS")
            .unwrap_or_else(|| DEFAULT_LLM_TIMEOUT_SECS.to_string()),
    );
    details.insert(
        "strictJsonMode".to_string(),
        read_local_env("VOICECODER_LLM_STRICT_JSON_MODE")
            .unwrap_or_else(|| DEFAULT_STRICT_JSON_MODE.to_string()),
    );
    details.insert(
        "apiKeyConfigured".to_string(),
        read_local_env("VOICECODER_LLM_API_KEY")
            .is_some()
            .to_string(),
    );
    details
}

fn required_env(key: &str) -> Result<String, String> {
    read_local_env(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("缺少本地环境变量 {key}，请先配置 LLM。"))
}

fn parse_f32_env(key: &str, default_value: f32) -> Result<f32, String> {
    read_local_env(key)
        .map(|value| {
            value
                .parse::<f32>()
                .map_err(|_| format!("{key} 必须是数字。"))
        })
        .unwrap_or(Ok(default_value))
}

fn parse_u64_env(key: &str, default_value: u64) -> Result<u64, String> {
    read_local_env(key)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{key} 必须是正整数。"))
        })
        .unwrap_or(Ok(default_value))
}

fn optional_bool_env(key: &str, default_value: bool) -> bool {
    read_local_env(key)
        .and_then(|value| match value.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default_value)
}

fn normalize_base_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("VOICECODER_LLM_BASE_URL 不能为空。".to_string());
    }

    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err("VOICECODER_LLM_BASE_URL 必须以 http:// 或 https:// 开头。".to_string());
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_override_parser_accepts_openai_compatible_and_auto() {
        assert_eq!(
            LlmProviderRegistry::parse_provider_override("openai_compatible"),
            Some(LlmProviderKind::OpenaiCompatible)
        );
        assert_eq!(LlmProviderRegistry::parse_provider_override("auto"), None);
        assert_eq!(LlmProviderRegistry::parse_provider_override("mock"), None);
    }

    #[test]
    fn normalizes_base_url_for_chat_completions() {
        assert_eq!(
            normalize_base_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1"
        );
        assert!(normalize_base_url("api.example.com/v1").is_err());
    }

    #[test]
    fn parses_json_content_from_plain_or_fenced_text() {
        assert_eq!(
            parse_json_object_content("{\"ok\":true}").unwrap()["ok"],
            Value::Bool(true)
        );
        assert_eq!(
            parse_json_object_content("```json\n{\"ok\":true}\n```").unwrap()["ok"],
            Value::Bool(true)
        );
        assert!(parse_json_object_content("[true]").is_err());
    }

    #[test]
    fn extracts_chat_completion_content() {
        let response = json!({
            "choices": [
                {
                    "message": {
                        "content": "{\"ok\":true}"
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completion_content(&response).unwrap(),
            "{\"ok\":true}"
        );
    }

    #[test]
    fn rejects_conflicting_processing_result() {
        let value = json!({
            "summary": "做一个语音需求整理流程",
            "requirementDocumentDraft": "目标：实现语音需求整理。",
            "confirmedFacts": [],
            "constraints": [],
            "acceptanceCriteria": ["用户点击确认前不编码"],
            "outOfScope": [],
            "risks": [],
            "questions": [
                {
                    "question": "最重要验收标准是什么？",
                    "reason": "需要验收标准",
                    "blocksCoding": true
                }
            ],
            "readyToConfirm": true
        });

        assert!(parse_requirement_processing_result(value).is_err());
    }

    #[tokio::test]
    #[ignore = "uses real VOICECODER_LLM_* env and network"]
    async fn openai_compatible_health_check_from_env() {
        let result = test_llm_provider_connection().await;
        assert!(result.ok, "{:?}", result.error);
    }
}
