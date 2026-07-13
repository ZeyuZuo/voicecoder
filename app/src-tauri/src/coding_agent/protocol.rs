//! Codex notification and `exec --json` normalization into Agent domain events.

use super::model::{AgentEvent, AgentPlanStep};
use super::{
    validate_request_id, AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS, AGENT_UI_LONG_TEXT_CHARS,
    AGENT_UI_MODEL_IDENTIFIER_CHARS, AGENT_UI_MODEL_STATUS_ITEMS, AGENT_UI_PLAN_STEPS,
    AGENT_UI_REASONING_PARTS, AGENT_UI_STATUS_MESSAGE_CHARS, AGENT_UI_TEXT_PREVIEW_CHARS,
};
use chrono::{TimeZone, Utc};
use serde_json::{json, Map, Value};

mod ui_projection;
pub(super) use ui_projection::*;

#[allow(dead_code)]
pub(super) fn normalize_codex_notification(notification: &Value) -> Vec<AgentEvent> {
    normalize_codex_notification_at(notification, &current_agent_event_timestamp())
}

pub(super) fn normalize_codex_notification_at(
    notification: &Value,
    created_at: &str,
) -> Vec<AgentEvent> {
    let Some(method) = notification.get("method").and_then(Value::as_str) else {
        return Vec::new();
    };
    let params = notification.get("params").unwrap_or(&Value::Null);

    match method {
        "thread/started" => extract_string(params, "/thread/id")
            .map(|thread_id| {
                vec![AgentEvent::ThreadStarted {
                    thread_id,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "turn/started" => vec![AgentEvent::TurnStarted {
            turn_id: extract_string(params, "/turn/id"),
            created_at: created_at.to_string(),
        }],
        "hook/started" => normalize_codex_hook_run(params, created_at, false),
        "hook/completed" => normalize_codex_hook_run(params, created_at, true),
        "item/agentMessage/delta"
        | "item/plan/delta"
        | "item/commandExecution/outputDelta"
        | "item/commandExecution/terminalInteraction"
        | "item/fileChange/outputDelta"
        | "item/fileChange/patchUpdated"
        | "item/mcpToolCall/progress"
        | "item/reasoning/summaryTextDelta"
        | "item/reasoning/summaryPartAdded"
        | "item/reasoning/textDelta" => normalize_codex_item_delta(method, params, created_at),
        "turn/plan/updated" => normalize_codex_plan_updated(params, created_at),
        "turn/diff/updated" => {
            let thread_id = extract_string(params, "/threadId");
            let turn_id = extract_string(params, "/turnId");
            let diff = extract_string(params, "/diff");
            match (thread_id, turn_id, diff) {
                (Some(thread_id), Some(turn_id), Some(diff)) => {
                    vec![AgentEvent::TurnDiffUpdated {
                        thread_id,
                        turn_id,
                        diff,
                        created_at: created_at.to_string(),
                    }]
                }
                _ => Vec::new(),
            }
        }
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed" => {
            normalize_auto_approval_review(params, created_at)
        }
        "serverRequest/resolved" => params
            .get("requestId")
            .filter(|request_id| validate_request_id(request_id).is_ok())
            .map(|request_id| {
                vec![server_request_resolved_event(
                    request_id.clone(),
                    "resolved".to_string(),
                    Some("server".to_string()),
                    Some("Codex 已完成该请求的处理".to_string()),
                    created_at.to_string(),
                )]
            })
            .unwrap_or_default(),
        "item/started" => normalize_codex_item_lifecycle(params, created_at, false),
        "item/completed" => normalize_codex_item_lifecycle(params, created_at, true),
        "thread/compacted" => normalize_codex_context_compacted(params, created_at),
        "thread/tokenUsage/updated" => normalize_codex_token_usage(params, created_at),
        "model/rerouted" => normalize_codex_model_rerouted(params, created_at),
        "model/safetyBuffering/updated" => {
            normalize_codex_model_safety_buffering(params, created_at)
        }
        "model/verification" => normalize_codex_model_verification(params, created_at),
        "turn/completed" => normalize_codex_turn_completed(params, created_at),
        "warning" => extract_string(params, "/message")
            .map(|message| {
                vec![AgentEvent::Warning {
                    message: project_ui_sensitive_text(&message, AGENT_UI_STATUS_MESSAGE_CHARS),
                    thread_id: extract_string(params, "/threadId"),
                    turn_id: extract_string(params, "/turnId"),
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "configWarning" => normalize_codex_config_warning(params, created_at),
        "guardianWarning" => normalize_codex_guardian_warning(params, created_at),
        "error" | "thread/realtime/error" => vec![AgentEvent::Error {
            message: project_ui_sensitive_text(
                &format_codex_error(params.get("error").unwrap_or(params)),
                AGENT_UI_STATUS_MESSAGE_CHARS,
            ),
            retryable: params
                .get("willRetry")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            terminal: !params
                .get("willRetry")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            thread_id: extract_string(params, "/threadId"),
            turn_id: extract_string(params, "/turnId"),
            created_at: created_at.to_string(),
        }],
        _ => Vec::new(),
    }
}

pub(super) fn normalize_codex_exec_json_event(event: &Value) -> Vec<AgentEvent> {
    normalize_codex_exec_json_event_at(event, &current_agent_event_timestamp())
}

pub(super) fn normalize_codex_exec_json_event_at(
    event: &Value,
    created_at: &str,
) -> Vec<AgentEvent> {
    let Some(event_type) = event.get("type").and_then(Value::as_str) else {
        return Vec::new();
    };

    match event_type {
        "thread.started" => extract_string(event, "/thread_id")
            .or_else(|| extract_string(event, "/thread/id"))
            .map(|thread_id| {
                vec![AgentEvent::ThreadStarted {
                    thread_id,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "turn.started" => vec![AgentEvent::TurnStarted {
            turn_id: extract_string(event, "/turn_id")
                .or_else(|| extract_string(event, "/turn/id")),
            created_at: created_at.to_string(),
        }],
        "turn.completed" => vec![AgentEvent::TurnCompleted {
            thread_id: extract_string(event, "/thread_id")
                .or_else(|| extract_string(event, "/threadId")),
            turn_id: extract_string(event, "/turn_id").or_else(|| extract_string(event, "/turnId")),
            status: "completed".to_string(),
            final_message: extract_exec_final_message(event),
            created_at: created_at.to_string(),
        }],
        "turn.failed" => vec![AgentEvent::Error {
            message: extract_string(event, "/error")
                .or_else(|| extract_string(event, "/message"))
                .or_else(|| {
                    event
                        .get("error")
                        .map(format_codex_error)
                        .filter(|message| !message.is_empty())
                })
                .unwrap_or_else(|| "Codex exec --json turn failed.".to_string()),
            retryable: false,
            terminal: true,
            thread_id: extract_string(event, "/thread_id")
                .or_else(|| extract_string(event, "/threadId")),
            turn_id: extract_string(event, "/turn_id").or_else(|| extract_string(event, "/turnId")),
            created_at: created_at.to_string(),
        }],
        "item.started" | "item.completed" => {
            normalize_codex_exec_json_item(event.get("item"), created_at)
        }
        "item.agent_message.delta" | "item.agentMessage.delta" => extract_string(event, "/delta")
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![AgentEvent::AgentMessage {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "error" => vec![AgentEvent::Error {
            message: event
                .get("error")
                .map(format_codex_error)
                .unwrap_or_else(|| format_codex_error(event)),
            retryable: false,
            terminal: true,
            thread_id: extract_string(event, "/thread_id")
                .or_else(|| extract_string(event, "/threadId")),
            turn_id: extract_string(event, "/turn_id").or_else(|| extract_string(event, "/turnId")),
            created_at: created_at.to_string(),
        }],
        _ => Vec::new(),
    }
}

pub(super) fn normalize_codex_item_lifecycle(
    params: &Value,
    created_at: &str,
    completed: bool,
) -> Vec<AgentEvent> {
    let Some(item) = params.get("item") else {
        return Vec::new();
    };
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(item_id) = extract_string(item, "/id") else {
        return Vec::new();
    };
    let item_type = extract_string(item, "/type").unwrap_or_else(|| "unknown".to_string());
    let status = extract_string(item, "/status");
    let projected_item = project_codex_item_for_ui(item, &item_type);

    if completed {
        vec![AgentEvent::ItemCompleted {
            thread_id,
            turn_id,
            item_id,
            item_type,
            lifecycle: "completed".to_string(),
            status,
            completed_at: extract_protocol_timestamp(params, "/completedAtMs", created_at),
            item: projected_item,
            created_at: created_at.to_string(),
        }]
    } else {
        vec![AgentEvent::ItemStarted {
            thread_id,
            turn_id,
            item_id,
            item_type,
            lifecycle: "in_progress".to_string(),
            status,
            started_at: extract_protocol_timestamp(params, "/startedAtMs", created_at),
            item: projected_item,
            created_at: created_at.to_string(),
        }]
    }
}

pub(super) fn normalize_codex_hook_run(
    params: &Value,
    created_at: &str,
    completed: bool,
) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(run) = params.get("run") else {
        return Vec::new();
    };
    let Some(hook_id) = extract_string(run, "/id") else {
        return Vec::new();
    };

    vec![AgentEvent::HookRunUpdated {
        thread_id,
        turn_id: extract_string(params, "/turnId"),
        hook_id,
        lifecycle: if completed {
            "completed".to_string()
        } else {
            "in_progress".to_string()
        },
        run: project_codex_hook_run_for_ui(run),
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_context_compacted(
    params: &Value,
    created_at: &str,
) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };

    vec![AgentEvent::ContextCompacted {
        thread_id,
        turn_id,
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_token_usage(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(token_usage) = params.get("tokenUsage") else {
        return Vec::new();
    };

    vec![AgentEvent::TokenUsageUpdated {
        thread_id,
        turn_id,
        token_usage: project_token_usage_for_ui(token_usage),
        created_at: created_at.to_string(),
    }]
}

pub(super) fn project_token_usage_for_ui(token_usage: &Value) -> Value {
    let mut projected = Map::new();
    for key in ["total", "last"] {
        let Some(breakdown) = token_usage.get(key) else {
            continue;
        };
        let mut projected_breakdown = Map::new();
        for field in [
            "totalTokens",
            "inputTokens",
            "cachedInputTokens",
            "outputTokens",
            "reasoningOutputTokens",
        ] {
            if let Some(value) = breakdown.get(field).filter(|value| value.is_number()) {
                projected_breakdown.insert(field.to_string(), value.clone());
            }
        }
        projected.insert(key.to_string(), Value::Object(projected_breakdown));
    }
    if let Some(context_window) = token_usage.get("modelContextWindow") {
        if context_window.is_number() || context_window.is_null() {
            projected.insert("modelContextWindow".to_string(), context_window.clone());
        }
    }
    Value::Object(projected)
}

pub(super) fn normalize_codex_model_rerouted(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(from_model) = extract_string(params, "/fromModel") else {
        return Vec::new();
    };
    let Some(to_model) = extract_string(params, "/toModel") else {
        return Vec::new();
    };
    let Some(reason) = extract_string(params, "/reason") else {
        return Vec::new();
    };

    vec![AgentEvent::ModelRerouted {
        thread_id,
        turn_id,
        from_model: truncate_ui_text(&from_model, AGENT_UI_MODEL_IDENTIFIER_CHARS),
        to_model: truncate_ui_text(&to_model, AGENT_UI_MODEL_IDENTIFIER_CHARS),
        reason: project_ui_sensitive_text(&reason, AGENT_UI_TEXT_PREVIEW_CHARS),
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_model_safety_buffering(
    params: &Value,
    created_at: &str,
) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(model) = extract_string(params, "/model") else {
        return Vec::new();
    };
    let Some(use_cases) = extract_bounded_string_array(
        params,
        "/useCases",
        AGENT_UI_MODEL_STATUS_ITEMS,
        AGENT_UI_TEXT_PREVIEW_CHARS,
    ) else {
        return Vec::new();
    };
    let Some(reasons) = extract_bounded_string_array(
        params,
        "/reasons",
        AGENT_UI_MODEL_STATUS_ITEMS,
        AGENT_UI_TEXT_PREVIEW_CHARS,
    ) else {
        return Vec::new();
    };
    let Some(show_buffering_ui) = params.get("showBufferingUi").and_then(Value::as_bool) else {
        return Vec::new();
    };

    vec![AgentEvent::ModelSafetyBufferingUpdated {
        thread_id,
        turn_id,
        model: truncate_ui_text(&model, AGENT_UI_MODEL_IDENTIFIER_CHARS),
        use_cases,
        reasons,
        show_buffering_ui,
        faster_model: extract_string(params, "/fasterModel")
            .map(|value| truncate_ui_text(&value, AGENT_UI_MODEL_IDENTIFIER_CHARS)),
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_model_verification(
    params: &Value,
    created_at: &str,
) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(verifications) = extract_bounded_string_array(
        params,
        "/verifications",
        AGENT_UI_MODEL_STATUS_ITEMS,
        AGENT_UI_TEXT_PREVIEW_CHARS,
    ) else {
        return Vec::new();
    };

    vec![AgentEvent::ModelVerificationUpdated {
        thread_id,
        turn_id,
        verifications,
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_config_warning(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(summary) = extract_string(params, "/summary") else {
        return Vec::new();
    };

    vec![AgentEvent::ConfigWarning {
        summary: project_ui_sensitive_text(&summary, AGENT_UI_TEXT_PREVIEW_CHARS),
        details: extract_string(params, "/details")
            .map(|value| project_ui_sensitive_text(&value, AGENT_UI_STATUS_MESSAGE_CHARS)),
        path: extract_string(params, "/path")
            .map(|value| truncate_ui_text(&value, AGENT_UI_TEXT_PREVIEW_CHARS)),
        range: params.get("range").and_then(project_config_text_range),
        created_at: created_at.to_string(),
    }]
}

pub(super) fn project_config_text_range(range: &Value) -> Option<Value> {
    let mut projected = Map::new();
    for key in ["start", "end"] {
        let position = range.get(key)?;
        let mut projected_position = Map::new();
        for field in ["line", "column"] {
            if let Some(value) = position.get(field).filter(|value| value.is_number()) {
                projected_position.insert(field.to_string(), value.clone());
            }
        }
        projected.insert(key.to_string(), Value::Object(projected_position));
    }
    Some(Value::Object(projected))
}

pub(super) fn normalize_codex_guardian_warning(
    params: &Value,
    created_at: &str,
) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(message) = extract_string(params, "/message") else {
        return Vec::new();
    };

    vec![AgentEvent::GuardianWarning {
        thread_id,
        message: project_ui_sensitive_text(&message, AGENT_UI_STATUS_MESSAGE_CHARS),
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_item_delta(
    method: &str,
    params: &Value,
    created_at: &str,
) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(item_id) = extract_string(params, "/itemId") else {
        return Vec::new();
    };
    let item_type = match method {
        "item/agentMessage/delta" => "agentMessage",
        "item/plan/delta" => "plan",
        "item/commandExecution/outputDelta" | "item/commandExecution/terminalInteraction" => {
            "commandExecution"
        }
        "item/fileChange/outputDelta" | "item/fileChange/patchUpdated" => "fileChange",
        "item/mcpToolCall/progress" => "mcpToolCall",
        "item/reasoning/summaryTextDelta"
        | "item/reasoning/summaryPartAdded"
        | "item/reasoning/textDelta" => "reasoning",
        _ => "unknown",
    };
    let Some(delta) = normalize_codex_item_delta_payload(method, params) else {
        return Vec::new();
    };

    vec![AgentEvent::ItemDelta {
        thread_id,
        turn_id,
        item_id,
        item_type: item_type.to_string(),
        lifecycle: "in_progress".to_string(),
        method: method.to_string(),
        delta,
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_item_delta_payload(method: &str, params: &Value) -> Option<Value> {
    match method {
        "item/fileChange/patchUpdated" => params
            .get("changes")
            .cloned()
            .or_else(|| Some(Value::Array(Vec::new()))),
        "item/reasoning/summaryTextDelta" => {
            let summary_index = params.get("summaryIndex")?.as_u64()?;
            if summary_index >= AGENT_UI_REASONING_PARTS as u64 {
                return None;
            }
            Some(json!({
                "text": truncate_ui_text(
                    &extract_string(params, "/delta")?,
                    AGENT_UI_TEXT_PREVIEW_CHARS,
                ),
                "summaryIndex": summary_index,
                "visibility": "summary"
            }))
        }
        "item/reasoning/summaryPartAdded" => {
            let summary_index = params.get("summaryIndex")?.as_u64()?;
            if summary_index >= AGENT_UI_REASONING_PARTS as u64 {
                return None;
            }
            Some(json!({
                "summaryIndex": summary_index,
                "visibility": "summary"
            }))
        }
        "item/reasoning/textDelta" => {
            let text_length = extract_string(params, "/delta")?.chars().count();
            Some(json!({
                "contentIndex": params.get("contentIndex")?.as_u64()?,
                "textLength": text_length,
                "visibility": "restricted_debug"
            }))
        }
        "item/mcpToolCall/progress" => Some(json!({
            "message": project_ui_sensitive_text(
                &extract_string(params, "/message")?,
                AGENT_UI_TEXT_PREVIEW_CHARS,
            )
        })),
        "item/commandExecution/terminalInteraction" => {
            let interaction_length = extract_string(params, "/stdin")?.chars().count();
            Some(json!({
                "processId": truncate_ui_text(
                    &extract_string(params, "/processId")?,
                    AGENT_UI_TEXT_PREVIEW_CHARS,
                ),
                "interaction": {
                    "type": "stdin",
                    "characterCount": interaction_length
                }
            }))
        }
        "item/agentMessage/delta" | "item/plan/delta" => Some(Value::String(truncate_ui_text(
            &extract_string(params, "/delta")?,
            AGENT_UI_LONG_TEXT_CHARS,
        ))),
        "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
            let (tail, _) = truncate_ui_text_tail(
                &extract_string(params, "/delta")?,
                AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS,
            );
            Some(Value::String(tail))
        }
        _ => Some(
            params
                .get("delta")
                .or_else(|| params.get("part"))
                .cloned()
                .unwrap_or_else(|| params.clone()),
        ),
    }
}

pub(super) fn normalize_codex_plan_updated(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let plan = params
        .get("plan")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .take(AGENT_UI_PLAN_STEPS)
                .filter_map(|step| {
                    Some(AgentPlanStep {
                        step: truncate_ui_text(
                            &extract_string(step, "/step")?,
                            AGENT_UI_TEXT_PREVIEW_CHARS,
                        ),
                        status: truncate_ui_text(
                            &extract_string(step, "/status")
                                .unwrap_or_else(|| "pending".to_string()),
                            AGENT_UI_MODEL_IDENTIFIER_CHARS,
                        ),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    vec![AgentEvent::PlanUpdated {
        thread_id,
        turn_id,
        explanation: extract_string(params, "/explanation")
            .filter(|value| !value.is_empty())
            .map(|value| truncate_ui_text(&value, AGENT_UI_STATUS_MESSAGE_CHARS)),
        plan,
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_exec_json_item(
    item: Option<&Value>,
    created_at: &str,
) -> Vec<AgentEvent> {
    let Some(item) = item else {
        return Vec::new();
    };

    match item.get("type").and_then(Value::as_str) {
        Some("agent_message") | Some("agentMessage") => extract_string(item, "/text")
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![AgentEvent::AgentMessage {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        Some("plan_update") | Some("plan") => format_exec_plan_update(item)
            .map(|text| {
                vec![AgentEvent::PlanUpdate {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        Some("command_execution") | Some("commandExecution") => {
            let Some(command) = extract_string(item, "/command") else {
                return Vec::new();
            };
            vec![AgentEvent::Command {
                command,
                status: extract_string(item, "/status").unwrap_or_else(|| "unknown".to_string()),
                created_at: created_at.to_string(),
            }]
        }
        Some("file_change") | Some("fileChange") => {
            normalize_codex_exec_json_file_change(item, created_at)
        }
        _ => Vec::new(),
    }
}

pub(super) fn normalize_codex_file_changes(
    changes: Option<&Value>,
    created_at: &str,
) -> Vec<AgentEvent> {
    changes
        .and_then(Value::as_array)
        .map(|changes| {
            changes
                .iter()
                .filter_map(|change| {
                    extract_string(change, "/path").map(|path| AgentEvent::FileChange {
                        path,
                        change_type: extract_string(change, "/kind/type"),
                        created_at: created_at.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn normalize_auto_approval_review(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(status) = extract_string(params, "/review/status") else {
        return Vec::new();
    };

    vec![AgentEvent::ApprovalReview {
        status: truncate_ui_text(&status, AGENT_UI_MODEL_IDENTIFIER_CHARS),
        action: extract_string(params, "/action/type")
            .map(|value| truncate_ui_text(&value, AGENT_UI_MODEL_IDENTIFIER_CHARS)),
        rationale: extract_string(params, "/review/rationale")
            .filter(|value| !value.is_empty())
            .map(|value| project_ui_sensitive_text(&value, AGENT_UI_STATUS_MESSAGE_CHARS)),
        created_at: created_at.to_string(),
    }]
}

pub(super) fn normalize_codex_exec_json_file_change(
    item: &Value,
    created_at: &str,
) -> Vec<AgentEvent> {
    let mut events = normalize_codex_file_changes(item.get("changes"), created_at);
    if events.is_empty() {
        if let Some(path) = extract_string(item, "/path") {
            events.push(AgentEvent::FileChange {
                path,
                change_type: extract_string(item, "/kind/type")
                    .or_else(|| extract_string(item, "/change_type"))
                    .or_else(|| extract_string(item, "/changeType")),
                created_at: created_at.to_string(),
            });
        }
    }

    events
}

pub(super) fn normalize_codex_turn_completed(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let status = truncate_ui_text(
        &extract_string(params, "/turn/status").unwrap_or_else(|| "completed".to_string()),
        AGENT_UI_MODEL_IDENTIFIER_CHARS,
    );
    let mut events = vec![AgentEvent::TurnCompleted {
        thread_id: extract_string(params, "/threadId"),
        turn_id: extract_string(params, "/turn/id"),
        status: status.clone(),
        final_message: extract_final_agent_message(params)
            .map(|message| truncate_ui_text(&message, AGENT_UI_LONG_TEXT_CHARS)),
        created_at: created_at.to_string(),
    }];

    if let Some(error) = params
        .pointer("/turn/error")
        .filter(|error| !error.is_null())
    {
        events.push(AgentEvent::Error {
            message: project_ui_sensitive_text(
                &format_codex_error(error),
                AGENT_UI_STATUS_MESSAGE_CHARS,
            ),
            retryable: false,
            terminal: status == "failed",
            thread_id: extract_string(params, "/threadId"),
            turn_id: extract_string(params, "/turn/id"),
            created_at: created_at.to_string(),
        });
    }

    events
}

pub(super) fn extract_final_agent_message(params: &Value) -> Option<String> {
    params
        .pointer("/turn/items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .rev()
                .find(|item| {
                    item.get("type").and_then(Value::as_str) == Some("agentMessage")
                        && item.get("phase").and_then(Value::as_str) == Some("final_answer")
                })
                .or_else(|| {
                    items.iter().rev().find(|item| {
                        item.get("type").and_then(Value::as_str) == Some("agentMessage")
                    })
                })
                .and_then(|item| extract_string(item, "/text").filter(|text| !text.is_empty()))
        })
}

pub(super) fn extract_exec_final_message(event: &Value) -> Option<String> {
    extract_string(event, "/final_message")
        .or_else(|| extract_string(event, "/finalMessage"))
        .or_else(|| extract_string(event, "/message"))
        .or_else(|| extract_final_agent_message(event))
}

pub(super) fn format_turn_plan_update(params: &Value) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(explanation) =
        extract_string(params, "/explanation").filter(|text| !text.is_empty())
    {
        lines.push(truncate_ui_text(
            &explanation,
            AGENT_UI_STATUS_MESSAGE_CHARS,
        ));
    }

    if let Some(plan) = params.get("plan").and_then(Value::as_array) {
        lines.extend(plan.iter().take(AGENT_UI_PLAN_STEPS).filter_map(|step| {
            let step_text =
                truncate_ui_text(&extract_string(step, "/step")?, AGENT_UI_TEXT_PREVIEW_CHARS);
            let status = truncate_ui_text(
                &extract_string(step, "/status").unwrap_or_else(|| "pending".to_string()),
                AGENT_UI_MODEL_IDENTIFIER_CHARS,
            );
            Some(format!("[{status}] {step_text}"))
        }));
    }

    if lines.is_empty() {
        None
    } else {
        Some(truncate_ui_text(
            &lines.join("\n"),
            AGENT_UI_LONG_TEXT_CHARS,
        ))
    }
}

pub(super) fn format_exec_plan_update(item: &Value) -> Option<String> {
    extract_string(item, "/text")
        .or_else(|| extract_string(item, "/message"))
        .filter(|text| !text.is_empty())
        .map(|text| truncate_ui_text(&text, AGENT_UI_LONG_TEXT_CHARS))
        .or_else(|| format_turn_plan_update(item))
}

pub(super) fn format_codex_error(error: &Value) -> String {
    extract_string(error, "/message")
        .or_else(|| extract_string(error, "/error/message"))
        .or_else(|| extract_string(error, "/message/text"))
        .unwrap_or_else(|| error.to_string())
}

pub(super) fn extract_string(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

pub(super) fn extract_bounded_string_array(
    value: &Value,
    pointer: &str,
    max_items: usize,
    max_chars: usize,
) -> Option<Vec<String>> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(max_items)
                .filter_map(Value::as_str)
                .map(|item| project_ui_sensitive_text(item, max_chars))
                .collect()
        })
}

pub(super) fn extract_protocol_timestamp(value: &Value, pointer: &str, fallback: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_else(|| fallback.to_string())
}

pub(super) fn server_request_key(request_id: &Value) -> String {
    match request_id {
        Value::String(value) => format!("string:{value}"),
        Value::Number(value) => format!("number:{value}"),
        _ => format!("unknown:{request_id}"),
    }
}

pub(super) fn server_request_resolved_event(
    request_id: Value,
    status: String,
    resolution: Option<String>,
    message: Option<String>,
    created_at: String,
) -> AgentEvent {
    AgentEvent::ServerRequestResolved {
        request_key: server_request_key(&request_id),
        request_id,
        status,
        resolution,
        message,
        created_at,
    }
}

pub(super) fn current_agent_event_timestamp() -> String {
    Utc::now().to_rfc3339()
}
