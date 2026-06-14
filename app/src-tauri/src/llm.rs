use crate::env_config::read_local_env;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

const DEFAULT_LLM_TEMPERATURE: f32 = 0.2;
const DEFAULT_LLM_TIMEOUT_SECS: u64 = 30;
const DEFAULT_STRICT_JSON_MODE: bool = true;

pub(crate) trait LlmProvider {
    fn kind(&self) -> LlmProviderKind;
    fn validate_start(&self) -> Result<(), String>;
    fn diagnostic(&self) -> LlmProviderDiagnostic;
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
}
