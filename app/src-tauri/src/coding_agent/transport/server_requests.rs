//! Approval, user-input, and MCP server-request policies and typed responses.

use super::*;

pub(in crate::coding_agent) fn classify_server_request_handling(
    method: &str,
    params: &Value,
    approvals_reviewer: CodingAgentApprovalsReviewer,
) -> ServerRequestHandling {
    if is_approval_server_request(method) {
        return if approvals_reviewer == CodingAgentApprovalsReviewer::User {
            ServerRequestHandling::UserDecision
        } else {
            ServerRequestHandling::AutoReview
        };
    }
    match method {
        "item/tool/requestUserInput" => ServerRequestHandling::UserInput {
            auto_resolve: params
                .get("autoResolutionMs")
                .and_then(Value::as_u64)
                .is_some(),
        },
        "mcpServer/elicitation/request" => ServerRequestHandling::McpElicitation,
        _ => ServerRequestHandling::Unsupported,
    }
}

pub(in crate::coding_agent) fn is_approval_server_request(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "applyPatchApproval"
            | "execCommandApproval"
    )
}

pub(in crate::coding_agent) fn server_request_timeout(
    handling: ServerRequestHandling,
    params: &Value,
) -> Duration {
    match handling {
        ServerRequestHandling::AutoReview => APP_SERVER_AUTO_REVIEW_TIMEOUT,
        ServerRequestHandling::UserInput { auto_resolve: true } => params
            .get("autoResolutionMs")
            .and_then(Value::as_u64)
            .map(Duration::from_millis)
            .unwrap_or(APP_SERVER_USER_DECISION_TIMEOUT),
        ServerRequestHandling::UserDecision
        | ServerRequestHandling::UserInput {
            auto_resolve: false,
        }
        | ServerRequestHandling::McpElicitation => APP_SERVER_USER_DECISION_TIMEOUT,
        ServerRequestHandling::Unsupported => APP_SERVER_SERVER_REQUEST_TIMEOUT,
    }
}

pub(in crate::coding_agent) fn server_request_expiry_timestamp(timeout: Duration) -> String {
    let timeout = chrono::Duration::from_std(timeout).unwrap_or_else(|_| chrono::Duration::zero());
    (Utc::now() + timeout).to_rfc3339()
}

pub(in crate::coding_agent) fn build_server_request_event(
    request_id: Value,
    method: String,
    params: Value,
    handling: ServerRequestHandling,
    expires_at: String,
    created_at: String,
) -> AgentEvent {
    AgentEvent::ServerRequest {
        request_key: server_request_key(&request_id),
        request_id,
        kind: server_request_kind(&method).to_string(),
        status: if handling == ServerRequestHandling::AutoReview {
            "auto_reviewing".to_string()
        } else if handling == ServerRequestHandling::Unsupported {
            "unsupported".to_string()
        } else {
            "pending".to_string()
        },
        requires_user_input: handling.requires_user_input(),
        auto_review: handling == ServerRequestHandling::AutoReview,
        thread_id: extract_string(&params, "/threadId"),
        turn_id: extract_string(&params, "/turnId"),
        item_id: extract_string(&params, "/itemId"),
        details: project_server_request_details(&method, &params),
        method,
        expires_at,
        created_at,
    }
}

pub(in crate::coding_agent) fn server_request_kind(method: &str) -> &'static str {
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => "command_approval",
        "item/fileChange/requestApproval" | "applyPatchApproval" => "file_approval",
        "item/permissions/requestApproval" => "permissions_approval",
        "item/tool/requestUserInput" => "user_input",
        "mcpServer/elicitation/request" => "mcp_elicitation",
        _ => "unsupported",
    }
}

pub(in crate::coding_agent) fn project_server_request_details(
    method: &str,
    params: &Value,
) -> Value {
    let mut details = Map::new();
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => {
            copy_ui_sensitive_text(
                &mut details,
                params,
                "command",
                AGENT_UI_STATUS_MESSAGE_CHARS,
            );
            copy_ui_text(&mut details, params, "cwd", AGENT_UI_TEXT_PREVIEW_CHARS);
            copy_ui_sensitive_text(
                &mut details,
                params,
                "reason",
                AGENT_UI_STATUS_MESSAGE_CHARS,
            );
        }
        "item/fileChange/requestApproval" | "applyPatchApproval" => {
            copy_ui_sensitive_text(
                &mut details,
                params,
                "reason",
                AGENT_UI_STATUS_MESSAGE_CHARS,
            );
            copy_ui_text(
                &mut details,
                params,
                "grantRoot",
                AGENT_UI_TEXT_PREVIEW_CHARS,
            );
        }
        "item/permissions/requestApproval" => {
            copy_ui_sensitive_text(
                &mut details,
                params,
                "reason",
                AGENT_UI_STATUS_MESSAGE_CHARS,
            );
            copy_ui_text(&mut details, params, "cwd", AGENT_UI_TEXT_PREVIEW_CHARS);
            if let Some(permissions) = params.get("permissions") {
                details.insert(
                    "permissions".to_string(),
                    project_ui_safe_value(permissions),
                );
            }
        }
        "item/tool/requestUserInput" => {
            if let Some(questions) = params.get("questions") {
                details.insert("questions".to_string(), project_ui_safe_value(questions));
            }
            if let Some(auto_resolution_ms) = params.get("autoResolutionMs") {
                details.insert("autoResolutionMs".to_string(), auto_resolution_ms.clone());
            }
        }
        "mcpServer/elicitation/request" => {
            for key in ["serverName", "mode", "message", "url", "elicitationId"] {
                copy_ui_sensitive_text(&mut details, params, key, AGENT_UI_STATUS_MESSAGE_CHARS);
            }
            if let Some(schema) = params.get("requestedSchema") {
                details.insert("requestedSchema".to_string(), project_ui_safe_value(schema));
            }
        }
        _ => {}
    }
    Value::Object(details)
}

pub(in crate::coding_agent) fn build_server_request_response(
    request: &PendingServerRequest,
    resolution: &ServerRequestResolution,
) -> Result<BuiltServerRequestResponse, String> {
    if request.method == "item/tool/requestUserInput"
        && matches!(resolution.action.as_str(), "cancel" | "decline")
    {
        let response = build_json_rpc_error_response(
            request.request_id.clone(),
            -32003,
            "VoiceCoder 用户取消了输入请求。",
        );
        let (status, resolution_name, message) = server_request_action_outcome(&resolution.action);
        return Ok(BuiltServerRequestResponse {
            response,
            log_payload: json!({
                "id": request.request_id,
                "error": { "code": -32003, "message": "User input cancelled" }
            }),
            status: status.to_string(),
            resolution: resolution_name.to_string(),
            message: message.to_string(),
        });
    }
    let result = match request.method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let decision = match resolution.action.as_str() {
                "accept" | "submit" => "accept",
                "acceptForSession" => "acceptForSession",
                "decline" => "decline",
                "cancel" => "cancel",
                action => return Err(format!("审批请求不支持操作 `{action}`。")),
            };
            json!({ "decision": decision })
        }
        "item/permissions/requestApproval" => {
            build_permissions_approval_result(&request.params, resolution)?
        }
        "item/tool/requestUserInput" => build_tool_user_input_result(&request.params, resolution)?,
        "mcpServer/elicitation/request" => build_mcp_elicitation_result(resolution)?,
        "applyPatchApproval" | "execCommandApproval" => {
            let decision = match resolution.action.as_str() {
                "accept" | "submit" => "approved",
                "acceptForSession" => "approved_for_session",
                "decline" => "denied",
                "cancel" => "abort",
                action => return Err(format!("旧版审批请求不支持操作 `{action}`。")),
            };
            json!({ "decision": decision })
        }
        method => {
            return Ok(unsupported_server_request_response(
                request.request_id.clone(),
                method,
            ));
        }
    };
    let response = build_json_rpc_result_response(request.request_id.clone(), result);
    let (status, resolution_name, message) = server_request_action_outcome(&resolution.action);
    let log_payload = if matches!(
        request.method.as_str(),
        "item/tool/requestUserInput" | "mcpServer/elicitation/request"
    ) {
        json!({
            "id": request.request_id,
            "result": "[REDACTED_USER_INPUT]"
        })
    } else {
        response.clone()
    };
    Ok(BuiltServerRequestResponse {
        response,
        log_payload,
        status: status.to_string(),
        resolution: resolution_name.to_string(),
        message: message.to_string(),
    })
}

pub(in crate::coding_agent) fn build_permissions_approval_result(
    params: &Value,
    resolution: &ServerRequestResolution,
) -> Result<Value, String> {
    if matches!(resolution.action.as_str(), "decline" | "cancel") {
        return Ok(json!({
            "permissions": {},
            "scope": "turn",
            "strictAutoReview": false
        }));
    }
    if !matches!(
        resolution.action.as_str(),
        "accept" | "acceptForSession" | "submit"
    ) {
        return Err(format!("权限请求不支持操作 `{}`。", resolution.action));
    }
    let requested = params
        .get("permissions")
        .and_then(Value::as_object)
        .ok_or_else(|| "权限请求缺少 permissions。".to_string())?;
    let mut granted = Map::new();
    for key in ["network", "fileSystem"] {
        if let Some(value) = requested.get(key).filter(|value| !value.is_null()) {
            granted.insert(key.to_string(), value.clone());
        }
    }
    let scope = if resolution.action == "acceptForSession"
        || resolution.scope.as_deref() == Some("session")
    {
        "session"
    } else {
        "turn"
    };
    Ok(json!({
        "permissions": Value::Object(granted),
        "scope": scope,
        "strictAutoReview": false
    }))
}

pub(in crate::coding_agent) fn build_tool_user_input_result(
    params: &Value,
    resolution: &ServerRequestResolution,
) -> Result<Value, String> {
    if matches!(resolution.action.as_str(), "cancel" | "decline") {
        return Err("用户输入请求已取消。".to_string());
    }
    if !matches!(resolution.action.as_str(), "accept" | "submit") {
        return Err(format!("用户输入请求不支持操作 `{}`。", resolution.action));
    }
    let allowed_ids = params
        .get("questions")
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter_map(|question| extract_string(question, "/id"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if resolution.answers.len() > 16 {
        return Err("用户输入回答数量超过上限。".to_string());
    }
    let mut answers = Map::new();
    for (question_id, values) in &resolution.answers {
        if !allowed_ids.iter().any(|allowed| allowed == question_id) {
            return Err(format!("用户输入包含未知问题 `{question_id}`。"));
        }
        if values.len() > 16
            || values
                .iter()
                .any(|value| value.chars().count() > AGENT_UI_LONG_TEXT_CHARS)
        {
            return Err(format!("问题 `{question_id}` 的回答超过安全上限。"));
        }
        answers.insert(question_id.clone(), json!({ "answers": values }));
    }
    Ok(json!({ "answers": Value::Object(answers) }))
}

pub(in crate::coding_agent) fn build_mcp_elicitation_result(
    resolution: &ServerRequestResolution,
) -> Result<Value, String> {
    let action = match resolution.action.as_str() {
        "accept" | "submit" => "accept",
        "decline" => "decline",
        "cancel" => "cancel",
        action => return Err(format!("MCP elicitation 不支持操作 `{action}`。")),
    };
    if resolution
        .content
        .as_ref()
        .map(|content| content.to_string().chars().count() > AGENT_UI_LONG_TEXT_CHARS)
        .unwrap_or(false)
    {
        return Err("MCP elicitation 回答超过安全上限。".to_string());
    }
    Ok(json!({
        "action": action,
        "content": if action == "accept" {
            resolution.content.clone().unwrap_or(Value::Null)
        } else {
            Value::Null
        },
        "_meta": Value::Null
    }))
}

pub(in crate::coding_agent) fn server_request_action_outcome(
    action: &str,
) -> (&'static str, &'static str, &'static str) {
    match action {
        "accept" => ("resolved", "accepted", "已批准本次请求"),
        "acceptForSession" => ("resolved", "accepted_for_session", "已批准本次会话中的请求"),
        "decline" => ("declined", "declined", "已拒绝该请求"),
        "cancel" => ("cancelled", "cancelled", "已取消该请求"),
        "submit" => ("resolved", "submitted", "已提交回答"),
        _ => ("failed", "invalid", "请求响应无效"),
    }
}

pub(in crate::coding_agent) fn build_server_request_timeout_response(
    request: &PendingServerRequest,
) -> Result<BuiltServerRequestResponse, String> {
    if let ServerRequestHandling::UserInput { auto_resolve: true } = request.handling {
        let resolution = ServerRequestResolution {
            request_id: request.request_id.clone(),
            action: "submit".to_string(),
            answers: recommended_user_input_answers(&request.params),
            content: None,
            scope: None,
        };
        let mut built = build_server_request_response(request, &resolution)?;
        built.status = "auto_resolved".to_string();
        built.resolution = "recommended_defaults".to_string();
        built.message = "等待回答超时，已使用每个问题的推荐首选项继续".to_string();
        return Ok(built);
    }

    if request.handling == ServerRequestHandling::Unsupported {
        return Ok(unsupported_server_request_response(
            request.request_id.clone(),
            &request.method,
        ));
    }

    let resolution = ServerRequestResolution {
        request_id: request.request_id.clone(),
        action: "cancel".to_string(),
        answers: BTreeMap::new(),
        content: None,
        scope: None,
    };
    let mut built = match build_server_request_response(request, &resolution) {
        Ok(built) => built,
        Err(_) if request.method == "item/tool/requestUserInput" => BuiltServerRequestResponse {
            response: build_json_rpc_error_response(
                request.request_id.clone(),
                -32003,
                "VoiceCoder 等待用户输入超时，已取消请求。",
            ),
            log_payload: json!({
                "id": request.request_id,
                "error": { "code": -32003, "message": "User input timed out" }
            }),
            status: "timed_out".to_string(),
            resolution: "timeout".to_string(),
            message: "等待用户输入超时，已取消请求".to_string(),
        },
        Err(error) => return Err(error),
    };
    built.status = "timed_out".to_string();
    built.resolution = "timeout".to_string();
    built.message = if request.handling == ServerRequestHandling::AutoReview {
        "Codex 自动审批超时，已安全取消请求".to_string()
    } else {
        "等待用户决定超时，已安全取消请求".to_string()
    };
    Ok(built)
}

pub(in crate::coding_agent) fn recommended_user_input_answers(
    params: &Value,
) -> BTreeMap<String, Vec<String>> {
    params
        .get("questions")
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter_map(|question| {
                    let id = extract_string(question, "/id")?;
                    let label = question
                        .pointer("/options/0/label")
                        .and_then(Value::as_str)?;
                    Some((id, vec![label.to_string()]))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(in crate::coding_agent) fn unsupported_server_request_response(
    request_id: Value,
    method: &str,
) -> BuiltServerRequestResponse {
    let response = build_json_rpc_error_response(
        request_id,
        -32601,
        "VoiceCoder 不支持该 app-server 主动请求。",
    );
    BuiltServerRequestResponse {
        log_payload: response.clone(),
        response,
        status: "failed".to_string(),
        resolution: "unsupported".to_string(),
        message: format!("VoiceCoder 不支持请求 `{method}`，已安全拒绝"),
    }
}
