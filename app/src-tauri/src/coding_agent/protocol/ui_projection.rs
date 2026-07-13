//! Bounded, credential-aware projections from raw Codex payloads to UI-safe values.

use super::super::{
    AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS, AGENT_UI_HOOK_ENTRIES, AGENT_UI_HOOK_ENTRY_CHARS,
    AGENT_UI_HOOK_TOTAL_CHARS, AGENT_UI_LONG_TEXT_CHARS, AGENT_UI_REASONING_PARTS,
    AGENT_UI_STATUS_MESSAGE_CHARS, AGENT_UI_STRUCTURED_MAX_DEPTH, AGENT_UI_STRUCTURED_MAX_FIELDS,
    AGENT_UI_STRUCTURED_MAX_ITEMS, AGENT_UI_STRUCTURED_MAX_NODES, AGENT_UI_STRUCTURED_TOTAL_CHARS,
    AGENT_UI_TEXT_PREVIEW_CHARS,
};
use serde_json::{json, Map, Value};

pub(in crate::coding_agent) fn project_codex_item_for_ui(item: &Value, item_type: &str) -> Value {
    let mut projected = ui_item_base(item, item_type);

    match item_type {
        "userMessage" | "contextCompaction" => {}
        "hookPrompt" => {
            if let Some(fragments) = item.get("fragments") {
                projected.insert("fragments".to_string(), project_ui_safe_value(fragments));
            }
        }
        "agentMessage" => {
            copy_ui_text(&mut projected, item, "text", AGENT_UI_LONG_TEXT_CHARS);
            copy_ui_value(&mut projected, item, "phase");
        }
        "plan" => copy_ui_text(&mut projected, item, "text", AGENT_UI_LONG_TEXT_CHARS),
        "reasoning" => {
            if let Some(summary) = item.get("summary") {
                let (projected_summary, summary_truncated) = project_reasoning_summary(summary);
                projected.insert("summary".to_string(), projected_summary);
                if summary_truncated {
                    projected.insert("_uiProjectionTruncated".to_string(), Value::Bool(true));
                }
            }
            let content_count = item
                .get("content")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let raw_text_available = content_count > 0
                || item
                    .get("rawTextAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            projected.insert(
                "rawTextAvailable".to_string(),
                Value::Bool(raw_text_available),
            );
            projected.insert("contentCount".to_string(), json!(content_count));
        }
        "commandExecution" => {
            copy_ui_text(&mut projected, item, "command", AGENT_UI_LONG_TEXT_CHARS);
            copy_ui_text(&mut projected, item, "cwd", AGENT_UI_TEXT_PREVIEW_CHARS);
            copy_ui_text(
                &mut projected,
                item,
                "processId",
                AGENT_UI_TEXT_PREVIEW_CHARS,
            );
            copy_ui_value(&mut projected, item, "source");
            copy_ui_value(&mut projected, item, "status");
            if let Some(actions) = item.get("commandActions") {
                projected.insert("commandActions".to_string(), project_ui_safe_value(actions));
            }
            if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str) {
                let (tail, truncated) =
                    truncate_ui_text_tail(output, AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS);
                let restricted = is_ui_credential_text(&tail);
                projected.insert(
                    "aggregatedOutput".to_string(),
                    Value::String(project_ui_credential_text(
                        &tail,
                        AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS,
                    )),
                );
                projected.insert(
                    "aggregatedOutputTruncated".to_string(),
                    Value::Bool(truncated || restricted),
                );
            }
            copy_ui_value(&mut projected, item, "exitCode");
            copy_ui_value(&mut projected, item, "durationMs");
        }
        "fileChange" => {
            copy_ui_value(&mut projected, item, "status");
            if let Some(changes) = item.get("changes") {
                projected.insert("changes".to_string(), project_file_changes_for_ui(changes));
            }
        }
        "mcpToolCall" => project_mcp_tool_call_for_ui(item, &mut projected),
        "dynamicToolCall" => project_dynamic_tool_call_for_ui(item, &mut projected),
        "collabAgentToolCall" => {
            for key in ["tool", "status", "reasoningEffort"] {
                copy_ui_value(&mut projected, item, key);
            }
            for key in ["senderThreadId", "model"] {
                copy_ui_text(&mut projected, item, key, AGENT_UI_TEXT_PREVIEW_CHARS);
            }
            if let Some(prompt) = item.get("prompt").and_then(Value::as_str) {
                projected.insert(
                    "prompt".to_string(),
                    Value::String(project_ui_sensitive_text(
                        prompt,
                        AGENT_UI_TEXT_PREVIEW_CHARS,
                    )),
                );
            }
            if let Some(receiver_thread_ids) = item.get("receiverThreadIds") {
                projected.insert(
                    "receiverThreadIds".to_string(),
                    project_ui_safe_value(receiver_thread_ids),
                );
            }
            if let Some(agent_states) = item.get("agentsStates") {
                projected.insert(
                    "agentsStates".to_string(),
                    project_ui_safe_value(agent_states),
                );
            }
        }
        "subAgentActivity" => {
            copy_ui_value(&mut projected, item, "kind");
            for key in ["agentThreadId", "agentPath"] {
                copy_ui_text(&mut projected, item, key, AGENT_UI_TEXT_PREVIEW_CHARS);
            }
        }
        "webSearch" => {
            if let Some(query) = item.get("query").and_then(Value::as_str) {
                projected.insert(
                    "query".to_string(),
                    Value::String(project_ui_sensitive_text(
                        query,
                        AGENT_UI_TEXT_PREVIEW_CHARS,
                    )),
                );
            }
            if let Some(action) = item.get("action") {
                projected.insert("action".to_string(), project_web_search_action(action));
            }
        }
        "imageView" => copy_ui_text(&mut projected, item, "path", AGENT_UI_TEXT_PREVIEW_CHARS),
        "sleep" => copy_ui_value(&mut projected, item, "durationMs"),
        "imageGeneration" => {
            copy_ui_value(&mut projected, item, "status");
            copy_ui_sensitive_text(
                &mut projected,
                item,
                "revisedPrompt",
                AGENT_UI_TEXT_PREVIEW_CHARS,
            );
            copy_ui_text(
                &mut projected,
                item,
                "savedPath",
                AGENT_UI_TEXT_PREVIEW_CHARS,
            );
            let result_length = item
                .get("result")
                .and_then(Value::as_str)
                .map(str::len)
                .unwrap_or(0);
            projected.insert(
                "resultAvailable".to_string(),
                Value::Bool(result_length > 0),
            );
            projected.insert("resultLength".to_string(), json!(result_length));
        }
        "enteredReviewMode" | "exitedReviewMode" => {
            copy_ui_sensitive_text(&mut projected, item, "review", AGENT_UI_TEXT_PREVIEW_CHARS)
        }
        _ => merge_unknown_item_projection(item, &mut projected),
    }

    Value::Object(projected)
}

pub(in crate::coding_agent) fn ui_item_base(item: &Value, item_type: &str) -> Map<String, Value> {
    let mut projected = Map::new();
    if let Some(id) = item.get("id").and_then(Value::as_str) {
        projected.insert("id".to_string(), Value::String(id.to_string()));
    }
    projected.insert("type".to_string(), Value::String(item_type.to_string()));
    projected
}

pub(in crate::coding_agent) fn copy_ui_value(
    projected: &mut Map<String, Value>,
    item: &Value,
    key: &str,
) {
    if let Some(value) = item.get(key) {
        projected.insert(key.to_string(), value.clone());
    }
}

pub(in crate::coding_agent) fn copy_ui_text(
    projected: &mut Map<String, Value>,
    item: &Value,
    key: &str,
    max_chars: usize,
) {
    if let Some(value) = item.get(key).and_then(Value::as_str) {
        projected.insert(
            key.to_string(),
            Value::String(truncate_ui_text(value, max_chars)),
        );
    }
}

pub(in crate::coding_agent) fn copy_ui_sensitive_text(
    projected: &mut Map<String, Value>,
    item: &Value,
    key: &str,
    max_chars: usize,
) {
    if let Some(value) = item.get(key).and_then(Value::as_str) {
        projected.insert(
            key.to_string(),
            Value::String(project_ui_sensitive_text(value, max_chars)),
        );
    }
}

pub(in crate::coding_agent) fn project_reasoning_summary(summary: &Value) -> (Value, bool) {
    let Some(parts) = summary.as_array() else {
        return (Value::Array(Vec::new()), false);
    };
    let projected = Value::Array(
        parts
            .iter()
            .take(AGENT_UI_REASONING_PARTS)
            .filter_map(Value::as_str)
            .map(|part| Value::String(project_ui_sensitive_text(part, AGENT_UI_TEXT_PREVIEW_CHARS)))
            .collect(),
    );
    let truncated = parts.len() > AGENT_UI_REASONING_PARTS
        || parts.iter().any(|part| {
            part.as_str().is_some_and(|text| {
                ui_text_exceeds(text, AGENT_UI_TEXT_PREVIEW_CHARS)
                    || is_ui_binary_or_credential_text(text)
            })
        });
    (projected, truncated)
}

pub(in crate::coding_agent) fn project_file_changes_for_ui(changes: &Value) -> Value {
    let Some(changes) = changes.as_array() else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        changes
            .iter()
            .filter_map(|change| {
                let mut projected = Map::new();
                copy_ui_text(&mut projected, change, "path", AGENT_UI_LONG_TEXT_CHARS);
                if let Some(kind) = change.get("kind") {
                    projected.insert("kind".to_string(), project_file_change_kind(kind));
                }
                copy_ui_value(&mut projected, change, "diff");
                (!projected.is_empty()).then_some(Value::Object(projected))
            })
            .collect(),
    )
}

pub(in crate::coding_agent) fn project_file_change_kind(kind: &Value) -> Value {
    let Some(kind) = kind.as_object() else {
        return kind.clone();
    };
    let mut projected = Map::new();
    for key in ["type", "move_path", "movePath"] {
        if let Some(value) = kind.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(projected)
}

pub(in crate::coding_agent) fn project_mcp_tool_call_for_ui(
    item: &Value,
    projected: &mut Map<String, Value>,
) {
    for key in ["status", "durationMs"] {
        copy_ui_value(projected, item, key);
    }
    for key in ["server", "tool", "pluginId"] {
        copy_ui_text(projected, item, key, AGENT_UI_TEXT_PREVIEW_CHARS);
    }
    for key in ["arguments", "result"] {
        if let Some(value) = item.get(key).filter(|value| !value.is_null()) {
            projected.insert(key.to_string(), project_ui_safe_value(value));
        }
    }
    if let Some(error) = item.get("error").and_then(Value::as_object) {
        let mut projected_error = Map::new();
        if let Some(message) = error.get("message").and_then(Value::as_str) {
            let redacted_or_truncated = is_ui_binary_or_credential_text(message)
                || ui_text_exceeds(message, AGENT_UI_TEXT_PREVIEW_CHARS);
            projected_error.insert(
                "message".to_string(),
                Value::String(project_ui_sensitive_text(
                    message,
                    AGENT_UI_TEXT_PREVIEW_CHARS,
                )),
            );
            if redacted_or_truncated {
                projected_error.insert("_uiProjectionTruncated".to_string(), Value::Bool(true));
            }
        }
        if !projected_error.is_empty() {
            projected.insert("error".to_string(), Value::Object(projected_error));
        }
    }
    if let Some(app_context) = item.get("appContext").and_then(Value::as_object) {
        let mut projected_context = Map::new();
        for key in ["appName", "actionName"] {
            if let Some(value) = app_context.get(key).and_then(Value::as_str) {
                projected_context.insert(
                    key.to_string(),
                    Value::String(truncate_ui_text(value, AGENT_UI_TEXT_PREVIEW_CHARS)),
                );
            }
        }
        if !projected_context.is_empty() {
            projected.insert("appContext".to_string(), Value::Object(projected_context));
        }
    }
}

pub(in crate::coding_agent) fn project_dynamic_tool_call_for_ui(
    item: &Value,
    projected: &mut Map<String, Value>,
) {
    for key in ["status", "success", "durationMs"] {
        copy_ui_value(projected, item, key);
    }
    for key in ["namespace", "tool"] {
        copy_ui_text(projected, item, key, AGENT_UI_TEXT_PREVIEW_CHARS);
    }
    for key in ["arguments", "contentItems"] {
        if let Some(value) = item.get(key).filter(|value| !value.is_null()) {
            projected.insert(key.to_string(), project_ui_safe_value(value));
        }
    }
}

pub(in crate::coding_agent) fn project_web_search_action(action: &Value) -> Value {
    let Some(action) = action.as_object() else {
        return Value::Null;
    };
    let mut projected = Map::new();
    for key in ["type", "query", "url", "pattern"] {
        if let Some(value) = action.get(key).and_then(Value::as_str) {
            projected.insert(
                key.to_string(),
                Value::String(project_ui_sensitive_text(
                    value,
                    AGENT_UI_TEXT_PREVIEW_CHARS,
                )),
            );
        }
    }
    if let Some(queries) = action.get("queries").and_then(Value::as_array) {
        projected.insert(
            "queries".to_string(),
            Value::Array(
                queries
                    .iter()
                    .take(AGENT_UI_STRUCTURED_MAX_ITEMS)
                    .filter_map(Value::as_str)
                    .map(|query| {
                        Value::String(project_ui_sensitive_text(
                            query,
                            AGENT_UI_TEXT_PREVIEW_CHARS,
                        ))
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(projected)
}

pub(in crate::coding_agent) fn merge_unknown_item_projection(
    item: &Value,
    projected: &mut Map<String, Value>,
) {
    let Value::Object(safe) = project_ui_safe_value(item) else {
        return;
    };
    for (key, value) in safe {
        if !matches!(key.as_str(), "id" | "type") {
            projected.insert(key, value);
        }
    }
}

pub(in crate::coding_agent) fn project_codex_hook_run_for_ui(run: &Value) -> Value {
    let mut projected = Map::new();
    let mut projection_truncated = run.as_object().is_some_and(|fields| {
        fields.keys().any(|key| {
            !matches!(
                key.as_str(),
                "id" | "eventName"
                    | "handlerType"
                    | "executionMode"
                    | "scope"
                    | "sourcePath"
                    | "source"
                    | "displayOrder"
                    | "status"
                    | "statusMessage"
                    | "startedAt"
                    | "completedAt"
                    | "durationMs"
                    | "entries"
            )
        })
    });
    for key in [
        "id",
        "eventName",
        "handlerType",
        "executionMode",
        "scope",
        "sourcePath",
        "source",
        "status",
        "statusMessage",
    ] {
        if let Some(value) = run.get(key).and_then(Value::as_str) {
            let max_chars = if key == "statusMessage" {
                AGENT_UI_HOOK_ENTRY_CHARS
            } else {
                AGENT_UI_TEXT_PREVIEW_CHARS
            };
            let projected_value = if key == "statusMessage" {
                if ui_text_exceeds(value, max_chars) || is_ui_binary_or_credential_text(value) {
                    projection_truncated = true;
                }
                project_ui_sensitive_text(value, max_chars)
            } else {
                if ui_text_exceeds(value, max_chars) {
                    projection_truncated = true;
                }
                truncate_ui_text(value, max_chars)
            };
            projected.insert(key.to_string(), Value::String(projected_value));
        }
    }
    for key in ["displayOrder", "startedAt", "completedAt", "durationMs"] {
        if let Some(value) = run.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    if let Some(entries) = run.get("entries").and_then(Value::as_array) {
        if entries.len() > AGENT_UI_HOOK_ENTRIES {
            projection_truncated = true;
        }
        let mut remaining_entry_chars = AGENT_UI_HOOK_TOTAL_CHARS;
        let mut projected_entries = Vec::new();
        for entry in entries.iter().take(AGENT_UI_HOOK_ENTRIES) {
            if remaining_entry_chars == 0 {
                projection_truncated = true;
                break;
            }
            let mut projected_entry = Map::new();
            if entry.as_object().is_some_and(|fields| {
                fields
                    .keys()
                    .any(|key| !matches!(key.as_str(), "kind" | "text"))
            }) {
                projection_truncated = true;
            }
            if let Some(kind) = entry.get("kind").and_then(Value::as_str) {
                projected_entry.insert(
                    "kind".to_string(),
                    Value::String(truncate_ui_text(kind, AGENT_UI_TEXT_PREVIEW_CHARS)),
                );
            }
            if let Some(text) = entry.get("text").and_then(Value::as_str) {
                let max_chars = remaining_entry_chars.min(AGENT_UI_HOOK_ENTRY_CHARS);
                if ui_text_exceeds(text, max_chars) || is_ui_binary_or_credential_text(text) {
                    projection_truncated = true;
                }
                let projected_text = project_ui_sensitive_text(text, max_chars);
                remaining_entry_chars =
                    remaining_entry_chars.saturating_sub(projected_text.chars().count());
                projected_entry.insert("text".to_string(), Value::String(projected_text));
            }
            if !projected_entry.is_empty() {
                projected_entries.push(Value::Object(projected_entry));
            }
        }
        projected.insert("entries".to_string(), Value::Array(projected_entries));
    }
    if projection_truncated {
        projected.insert("_uiProjectionTruncated".to_string(), Value::Bool(true));
    }
    Value::Object(projected)
}

pub(in crate::coding_agent) struct UiProjectionBudget {
    pub(in crate::coding_agent) remaining_chars: usize,
    pub(in crate::coding_agent) remaining_nodes: usize,
    pub(in crate::coding_agent) truncated: bool,
}

pub(in crate::coding_agent) fn project_ui_safe_value(value: &Value) -> Value {
    let mut budget = UiProjectionBudget {
        remaining_chars: AGENT_UI_STRUCTURED_TOTAL_CHARS,
        remaining_nodes: AGENT_UI_STRUCTURED_MAX_NODES,
        truncated: false,
    };
    let projected = project_ui_safe_value_at(value, 0, &mut budget);
    if !budget.truncated {
        return projected;
    }
    match projected {
        Value::Object(mut fields) => {
            fields.insert("_uiProjectionTruncated".to_string(), Value::Bool(true));
            Value::Object(fields)
        }
        Value::Array(mut items) => {
            items.push(json!({ "_uiProjectionTruncated": true }));
            Value::Array(items)
        }
        value => json!({
            "preview": value,
            "_uiProjectionTruncated": true
        }),
    }
}

pub(in crate::coding_agent) fn project_ui_safe_value_at(
    value: &Value,
    depth: usize,
    budget: &mut UiProjectionBudget,
) -> Value {
    if budget.remaining_nodes == 0 || depth > AGENT_UI_STRUCTURED_MAX_DEPTH {
        budget.truncated = true;
        return Value::String("[truncated]".to_string());
    }
    budget.remaining_nodes -= 1;

    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) => {
            if is_ui_binary_or_credential_text(text) {
                budget.truncated = true;
                return Value::String("[redacted binary or credential data]".to_string());
            }
            let max_chars = budget.remaining_chars.min(AGENT_UI_TEXT_PREVIEW_CHARS);
            if max_chars == 0 {
                budget.truncated = true;
                return Value::String("[truncated]".to_string());
            }
            if ui_text_exceeds(text, max_chars) {
                budget.truncated = true;
            }
            let projected = truncate_ui_text(text, max_chars);
            budget.remaining_chars = budget
                .remaining_chars
                .saturating_sub(projected.chars().count());
            Value::String(projected)
        }
        Value::Array(items) => {
            let limit = items.len().min(AGENT_UI_STRUCTURED_MAX_ITEMS);
            let projected = items
                .iter()
                .take(limit)
                .map(|item| project_ui_safe_value_at(item, depth + 1, budget))
                .collect::<Vec<_>>();
            if items.len() > limit {
                budget.truncated = true;
            }
            Value::Array(projected)
        }
        Value::Object(fields) => {
            let mut projected = Map::new();
            let mut redacted_fields = 0usize;
            let mut omitted_fields = 0usize;
            for (index, (key, child)) in fields.iter().enumerate() {
                if index >= AGENT_UI_STRUCTURED_MAX_FIELDS || budget.remaining_nodes == 0 {
                    omitted_fields = fields.len().saturating_sub(index);
                    budget.truncated = true;
                    break;
                }
                if is_ui_sensitive_field(key) {
                    redacted_fields += 1;
                    budget.truncated = true;
                    continue;
                }
                if ui_text_exceeds(key, 128) {
                    budget.truncated = true;
                }
                projected.insert(
                    truncate_ui_text(key, 128),
                    project_ui_safe_value_at(child, depth + 1, budget),
                );
            }
            if redacted_fields > 0 {
                projected.insert("_redactedFieldCount".to_string(), json!(redacted_fields));
            }
            if omitted_fields > 0 {
                projected.insert("_omittedFieldCount".to_string(), json!(omitted_fields));
            }
            Value::Object(projected)
        }
    }
}

pub(in crate::coding_agent) fn is_ui_sensitive_field(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
    normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized.contains("authorization")
        || normalized.contains("cookie")
        || normalized.contains("apikey")
        || normalized.contains("credential")
        || normalized.contains("privatekey")
        || normalized.contains("signature")
        || normalized == "stdin"
        || normalized == "raw"
        || normalized.starts_with("rawtext")
        || normalized.starts_with("rawcontent")
}

pub(in crate::coding_agent) fn is_ui_binary_or_credential_text(value: &str) -> bool {
    let trimmed = value.trim();
    if is_ui_credential_text(trimmed) {
        return true;
    }
    let marker_window = trimmed
        .chars()
        .take(AGENT_UI_STATUS_MESSAGE_CHARS)
        .collect::<String>()
        .to_ascii_lowercase();
    if marker_window.contains("data:") && marker_window.contains(";base64,") {
        return true;
    }
    if trimmed.len() < 512 {
        return false;
    }
    let mut base64_chars = 0usize;
    let mut non_whitespace_chars = 0usize;
    let mut whitespace_chars = 0usize;
    for character in trimmed.chars().take(AGENT_UI_STATUS_MESSAGE_CHARS) {
        if character.is_whitespace() {
            whitespace_chars += 1;
            continue;
        }
        non_whitespace_chars += 1;
        if character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=') {
            base64_chars += 1;
        }
    }
    let total_chars = non_whitespace_chars + whitespace_chars;
    non_whitespace_chars >= 512
        && base64_chars.saturating_mul(100) / non_whitespace_chars >= 98
        && whitespace_chars.saturating_mul(100) / total_chars.max(1) <= 2
}

pub(in crate::coding_agent) fn is_ui_credential_text(value: &str) -> bool {
    let marker_window = value
        .trim()
        .chars()
        .take(AGENT_UI_STATUS_MESSAGE_CHARS)
        .collect::<String>()
        .to_ascii_lowercase();
    marker_window.contains("bearer ")
        || marker_window.contains("-----begin ") && marker_window.contains("private key-----")
        || contains_ui_secret_key_marker(&marker_window)
        || [
            "apikey=",
            "apikey:",
            "api_key=",
            "api_key:",
            "api-key=",
            "api-key:",
            "token=",
            "token:",
            "password=",
            "password:",
            "authorization=",
            "authorization:",
            "signature=",
            "x-amz-credential=",
            "x-amz-signature=",
            "x-goog-signature=",
        ]
        .iter()
        .any(|marker| marker_window.contains(marker))
}

pub(in crate::coding_agent) fn contains_ui_secret_key_marker(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(3).enumerate().any(|(index, marker)| {
        if marker != b"sk-" {
            return false;
        }
        let has_boundary =
            index == 0 || (!bytes[index - 1].is_ascii_alphanumeric() && bytes[index - 1] != b'_');
        let suffix_length = bytes[index + 3..]
            .iter()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(**character, b'_' | b'-')
            })
            .count();
        has_boundary && suffix_length >= 12
    })
}

pub(in crate::coding_agent) fn project_ui_sensitive_text(value: &str, max_chars: usize) -> String {
    if is_ui_binary_or_credential_text(value) {
        truncate_ui_text("[redacted credential or binary data]", max_chars)
    } else {
        truncate_ui_text(value, max_chars)
    }
}

pub(in crate::coding_agent) fn project_ui_credential_text(value: &str, max_chars: usize) -> String {
    if is_ui_credential_text(value) {
        truncate_ui_text("[redacted credential data]", max_chars)
    } else {
        truncate_ui_text(value, max_chars)
    }
}

pub(in crate::coding_agent) fn ui_text_exceeds(value: &str, max_chars: usize) -> bool {
    value.chars().nth(max_chars).is_some()
}

pub(in crate::coding_agent) fn truncate_ui_text(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let mut projected = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_none() {
        return projected;
    }
    if max_chars > 0 {
        projected.pop();
        projected.push('…');
    }
    projected
}

pub(in crate::coding_agent) fn truncate_ui_text_tail(
    value: &str,
    max_chars: usize,
) -> (String, bool) {
    if max_chars == 0 {
        return (String::new(), !value.is_empty());
    }
    let Some((start, _)) = value.char_indices().rev().nth(max_chars.saturating_sub(1)) else {
        return (value.to_string(), false);
    };
    (value[start..].to_string(), start > 0)
}
