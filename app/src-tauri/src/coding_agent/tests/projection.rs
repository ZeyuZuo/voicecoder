use super::super::*;

#[test]
fn normalizes_reasoning_parts_and_redacts_raw_reasoning_text() {
    let summary_part = normalize_codex_notification_at(
        &json!({
            "method": "item/reasoning/summaryPartAdded",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "reasoning-1",
                "summaryIndex": 1
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let summary_delta = normalize_codex_notification_at(
        &json!({
            "method": "item/reasoning/summaryTextDelta",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "reasoning-1",
                "summaryIndex": 1,
                "delta": "检查测试"
            }
        }),
        "2026-07-13T00:00:01Z",
    );
    let second_summary_delta = normalize_codex_notification_at(
        &json!({
            "method": "item/reasoning/summaryTextDelta",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "reasoning-1",
                "summaryIndex": 2,
                "delta": "准备修改"
            }
        }),
        "2026-07-13T00:00:02Z",
    );
    let raw_delta = normalize_codex_notification_at(
        &json!({
            "method": "item/reasoning/textDelta",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "reasoning-1",
                "contentIndex": 3,
                "delta": "raw-secret"
            }
        }),
        "2026-07-13T00:00:03Z",
    );

    assert!(matches!(
        summary_part.as_slice(),
        [AgentEvent::ItemDelta { delta, .. }]
            if delta == &json!({ "summaryIndex": 1, "visibility": "summary" })
    ));
    assert!(matches!(
        summary_delta.as_slice(),
        [AgentEvent::ItemDelta { delta, .. }]
            if delta == &json!({
                "text": "检查测试",
                "summaryIndex": 1,
                "visibility": "summary"
            })
    ));
    assert!(matches!(
        second_summary_delta.as_slice(),
        [AgentEvent::ItemDelta { delta, .. }]
            if delta == &json!({
                "text": "准备修改",
                "summaryIndex": 2,
                "visibility": "summary"
            })
    ));
    assert!(matches!(
        raw_delta.as_slice(),
        [AgentEvent::ItemDelta { delta, .. }]
            if delta == &json!({
                "contentIndex": 3,
                "textLength": 10,
                "visibility": "restricted_debug"
            })
    ));
    assert!(!serde_json::to_string(&raw_delta)
        .expect("raw reasoning event should serialize")
        .contains("raw-secret"));
}

#[test]
fn normalizes_mcp_progress_and_redacts_terminal_stdin() {
    let progress = normalize_codex_notification_at(
        &json!({
            "method": "item/mcpToolCall/progress",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "mcp-1",
                "message": "正在读取资源"
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let terminal = normalize_codex_notification_at(
        &json!({
            "method": "item/commandExecution/terminalInteraction",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "processId": "pty-42",
                "stdin": "super-secret-input"
            }
        }),
        "2026-07-13T00:00:01Z",
    );

    assert!(matches!(
        progress.as_slice(),
        [AgentEvent::ItemDelta { delta, .. }]
            if delta == &json!({ "message": "正在读取资源" })
    ));
    assert!(matches!(
        terminal.as_slice(),
        [AgentEvent::ItemDelta { delta, .. }]
            if delta == &json!({
                "processId": "pty-42",
                "interaction": {
                    "type": "stdin",
                    "characterCount": 18
                }
            })
    ));
    let serialized =
        serde_json::to_string(&terminal).expect("terminal interaction event should serialize");
    assert!(!serialized.contains("super-secret-input"));
    assert!(!serialized.contains("\"stdin\":"));
}

#[test]
fn normalizes_hook_run_lifecycle_notifications() {
    let started_run = json!({
        "id": "hook-1",
        "eventName": "preToolUse",
        "handlerType": "command",
        "executionMode": "sync",
        "scope": "thread",
        "sourcePath": "/tmp/demo/.codex/hooks.json",
        "source": "project",
        "displayOrder": 0,
        "status": "running",
        "statusMessage": "检查命令",
        "startedAt": 1783900800000_i64,
        "completedAt": null,
        "durationMs": null,
        "entries": []
    });
    let mut completed_run = started_run.clone();
    completed_run["status"] = json!("completed");
    completed_run["completedAt"] = json!(1783900800100_i64);
    completed_run["durationMs"] = json!(100);
    completed_run["entries"] = json!([{ "kind": "feedback", "text": "允许执行" }]);

    let started = normalize_codex_notification_at(
        &json!({
            "method": "hook/started",
            "params": {
                "threadId": "thread-1",
                "turnId": null,
                "run": started_run.clone()
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let completed = normalize_codex_notification_at(
        &json!({
            "method": "hook/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "run": completed_run.clone()
            }
        }),
        "2026-07-13T00:00:01Z",
    );

    assert_eq!(
        started,
        vec![AgentEvent::HookRunUpdated {
            thread_id: "thread-1".to_string(),
            turn_id: None,
            hook_id: "hook-1".to_string(),
            lifecycle: "in_progress".to_string(),
            run: started_run,
            created_at: "2026-07-13T00:00:00Z".to_string(),
        }]
    );
    assert_eq!(
        completed,
        vec![AgentEvent::HookRunUpdated {
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            hook_id: "hook-1".to_string(),
            lifecycle: "completed".to_string(),
            run: completed_run,
            created_at: "2026-07-13T00:00:01Z".to_string(),
        }]
    );
}

#[test]
fn hook_run_projection_caps_entries_and_preserves_timing_metadata() {
    let entries = (0..(AGENT_UI_HOOK_ENTRIES + 10))
        .map(|index| {
            json!({
                "kind": "feedback",
                "text": format!(
                    "{index}: {}",
                    "hook output ".repeat(AGENT_UI_HOOK_ENTRY_CHARS / 4)
                ),
                "secretToken": "HOOK_SECRET_MUST_NOT_CROSS_IPC"
            })
        })
        .collect::<Vec<_>>();
    let events = normalize_codex_notification_at(
        &json!({
            "method": "hook/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "run": {
                    "id": "hook-bounded",
                    "eventName": "postToolUse",
                    "handlerType": "command",
                    "executionMode": "sync",
                    "scope": "turn",
                    "sourcePath": "/tmp/demo/.codex/hooks.json",
                    "source": "project",
                    "displayOrder": 7,
                    "status": "completed",
                    "statusMessage": "完成",
                    "startedAt": 1000,
                    "completedAt": 1200,
                    "durationMs": 200,
                    "entries": entries,
                    "arbitraryPayload": "not projected"
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );

    let [AgentEvent::HookRunUpdated { run, .. }] = events.as_slice() else {
        panic!("hook lifecycle event");
    };
    let projected_entries = run
        .get("entries")
        .and_then(Value::as_array)
        .expect("projected hook entries");
    assert!(projected_entries.len() <= AGENT_UI_HOOK_ENTRIES);
    assert!(projected_entries.iter().all(|entry| {
        entry
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.chars().count() <= AGENT_UI_HOOK_ENTRY_CHARS)
    }));
    assert!(
        projected_entries
            .iter()
            .filter_map(|entry| entry.get("text").and_then(Value::as_str))
            .map(|text| text.chars().count())
            .sum::<usize>()
            <= AGENT_UI_HOOK_TOTAL_CHARS
    );
    assert_eq!(run.get("displayOrder").and_then(Value::as_u64), Some(7));
    assert_eq!(run.get("startedAt").and_then(Value::as_u64), Some(1000));
    assert_eq!(run.get("completedAt").and_then(Value::as_u64), Some(1200));
    assert_eq!(run.get("durationMs").and_then(Value::as_u64), Some(200));
    assert!(run.get("arbitraryPayload").is_none());
    assert_eq!(run.get("_uiProjectionTruncated"), Some(&Value::Bool(true)));
    assert!(!serde_json::to_string(&events)
        .expect("hook event should serialize")
        .contains("HOOK_SECRET_MUST_NOT_CROSS_IPC"));
}

#[test]
fn reasoning_and_model_status_collections_are_bounded() {
    let summary = (0..(AGENT_UI_REASONING_PARTS + 10))
        .map(|index| format!("{index}:{}", "s".repeat(AGENT_UI_TEXT_PREVIEW_CHARS + 20)))
        .collect::<Vec<_>>();
    let reasoning = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1783900800000_i64,
                "item": {
                    "id": "reasoning-bounded",
                    "type": "reasoning",
                    "summary": summary,
                    "content": []
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let out_of_range_delta = normalize_codex_notification_at(
        &json!({
            "method": "item/reasoning/summaryTextDelta",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "reasoning-bounded",
                "summaryIndex": AGENT_UI_REASONING_PARTS,
                "delta": "must be ignored"
            }
        }),
        "2026-07-13T00:00:01Z",
    );
    let many_status_values = (0..(AGENT_UI_MODEL_STATUS_ITEMS + 10))
        .map(|index| {
            format!(
                "{index}: {}",
                "model status ".repeat(AGENT_UI_TEXT_PREVIEW_CHARS / 4)
            )
        })
        .collect::<Vec<_>>();
    let model = normalize_codex_notification_at(
        &json!({
            "method": "model/safetyBuffering/updated",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "model": "model-b",
                "useCases": many_status_values,
                "reasons": ["policy-check"],
                "showBufferingUi": true,
                "fasterModel": null
            }
        }),
        "2026-07-13T00:00:02Z",
    );

    let [AgentEvent::ItemCompleted { item, .. }] = reasoning.as_slice() else {
        panic!("reasoning lifecycle event");
    };
    let projected_summary = item
        .get("summary")
        .and_then(Value::as_array)
        .expect("projected reasoning summary");
    assert_eq!(projected_summary.len(), AGENT_UI_REASONING_PARTS);
    assert!(projected_summary.iter().all(|part| {
        part.as_str()
            .is_some_and(|text| text.chars().count() <= AGENT_UI_TEXT_PREVIEW_CHARS)
    }));
    assert!(out_of_range_delta.is_empty());

    let [AgentEvent::ModelSafetyBufferingUpdated { use_cases, .. }] = model.as_slice() else {
        panic!("model buffering event");
    };
    assert_eq!(use_cases.len(), AGENT_UI_MODEL_STATUS_ITEMS);
    assert!(use_cases
        .iter()
        .all(|value| value.chars().count() <= AGENT_UI_TEXT_PREVIEW_CHARS));
}

#[test]
fn normalizes_context_token_usage_and_schema_model_statuses() {
    let token_usage = json!({
        "total": {
            "totalTokens": 120,
            "inputTokens": 80,
            "cachedInputTokens": 20,
            "outputTokens": 40,
            "reasoningOutputTokens": 10
        },
        "last": {
            "totalTokens": 30,
            "inputTokens": 20,
            "cachedInputTokens": 5,
            "outputTokens": 10,
            "reasoningOutputTokens": 3
        },
        "modelContextWindow": null
    });
    let context = normalize_codex_notification_at(
        &json!({
            "method": "thread/compacted",
            "params": { "threadId": "thread-1", "turnId": "turn-1" }
        }),
        "2026-07-13T00:00:00Z",
    );
    let usage = normalize_codex_notification_at(
        &json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "tokenUsage": token_usage.clone()
            }
        }),
        "2026-07-13T00:00:01Z",
    );
    let rerouted = normalize_codex_notification_at(
        &json!({
            "method": "model/rerouted",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "fromModel": "model-a",
                "toModel": "model-b",
                "reason": "highRiskCyberActivity"
            }
        }),
        "2026-07-13T00:00:02Z",
    );
    let buffering = normalize_codex_notification_at(
        &json!({
            "method": "model/safetyBuffering/updated",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "model": "model-b",
                "useCases": ["cyber", "future-use-case"],
                "reasons": ["policy-check"],
                "showBufferingUi": true,
                "fasterModel": "model-fast"
            }
        }),
        "2026-07-13T00:00:03Z",
    );
    let verification = normalize_codex_notification_at(
        &json!({
            "method": "model/verification",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "verifications": ["trustedAccessForCyber"]
            }
        }),
        "2026-07-13T00:00:04Z",
    );

    assert!(matches!(
        context.as_slice(),
        [AgentEvent::ContextCompacted {
            thread_id,
            turn_id,
            ..
        }] if thread_id == "thread-1" && turn_id == "turn-1"
    ));
    assert_eq!(
        usage,
        vec![AgentEvent::TokenUsageUpdated {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            token_usage,
            created_at: "2026-07-13T00:00:01Z".to_string(),
        }]
    );
    assert!(matches!(
        rerouted.as_slice(),
        [AgentEvent::ModelRerouted { reason, .. }] if reason == "highRiskCyberActivity"
    ));
    assert!(matches!(
        buffering.as_slice(),
        [AgentEvent::ModelSafetyBufferingUpdated {
            use_cases,
            reasons,
            show_buffering_ui: true,
            faster_model: Some(faster_model),
            ..
        }] if use_cases == &["cyber", "future-use-case"]
            && reasons == &["policy-check"]
            && faster_model == "model-fast"
    ));
    assert!(matches!(
        verification.as_slice(),
        [AgentEvent::ModelVerificationUpdated { verifications, .. }]
            if verifications == &["trustedAccessForCyber"]
    ));
}

#[test]
fn model_string_enums_remain_forward_compatible() {
    let rerouted = normalize_codex_notification_at(
        &json!({
            "method": "model/rerouted",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "fromModel": "model-a",
                "toModel": "model-b",
                "reason": "futureRoutingPolicy"
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let verification = normalize_codex_notification_at(
        &json!({
            "method": "model/verification",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "verifications": ["futureVerification"]
            }
        }),
        "2026-07-13T00:00:01Z",
    );

    assert!(matches!(
        rerouted.as_slice(),
        [AgentEvent::ModelRerouted { reason, .. }] if reason == "futureRoutingPolicy"
    ));
    assert!(matches!(
        verification.as_slice(),
        [AgentEvent::ModelVerificationUpdated { verifications, .. }]
            if verifications == &["futureVerification"]
    ));
}

#[test]
fn normalizes_config_and_guardian_warnings() {
    let config = normalize_codex_notification_at(
        &json!({
            "method": "configWarning",
            "params": {
                "summary": "配置项已弃用",
                "details": "请迁移到新配置项",
                "path": "/tmp/demo/.codex/config.toml",
                "range": {
                    "start": { "line": 3, "column": 1 },
                    "end": { "line": 3, "column": 12 }
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let guardian = normalize_codex_notification_at(
        &json!({
            "method": "guardianWarning",
            "params": {
                "threadId": "thread-1",
                "message": "该操作需要额外审查"
            }
        }),
        "2026-07-13T00:00:01Z",
    );

    assert_eq!(
        config,
        vec![AgentEvent::ConfigWarning {
            summary: "配置项已弃用".to_string(),
            details: Some("请迁移到新配置项".to_string()),
            path: Some("/tmp/demo/.codex/config.toml".to_string()),
            range: Some(json!({
                "start": { "line": 3, "column": 1 },
                "end": { "line": 3, "column": 12 }
            })),
            created_at: "2026-07-13T00:00:00Z".to_string(),
        }]
    );
    assert_eq!(
        guardian,
        vec![AgentEvent::GuardianWarning {
            thread_id: "thread-1".to_string(),
            message: "该操作需要额外审查".to_string(),
            created_at: "2026-07-13T00:00:01Z".to_string(),
        }]
    );
}

#[test]
fn all_thread_item_types_keep_lifecycle_events_after_ui_projection() {
    let items = vec![
        json!({
            "id": "user-1",
            "type": "userMessage",
            "clientId": null,
            "content": []
        }),
        json!({ "id": "hook-prompt-1", "type": "hookPrompt", "fragments": [] }),
        json!({
            "id": "message-1",
            "type": "agentMessage",
            "text": "完成",
            "phase": "final_answer",
            "memoryCitation": null
        }),
        json!({ "id": "plan-1", "type": "plan", "text": "实现功能" }),
        json!({
            "id": "reasoning-1",
            "type": "reasoning",
            "summary": ["检查实现"],
            "content": ["restricted"]
        }),
        json!({
            "id": "command-1",
            "type": "commandExecution",
            "command": "npm test",
            "cwd": "/tmp/demo",
            "processId": null,
            "source": "agent",
            "status": "completed",
            "commandActions": [],
            "aggregatedOutput": "ok",
            "exitCode": 0,
            "durationMs": 15
        }),
        json!({
            "id": "file-1",
            "type": "fileChange",
            "status": "completed",
            "changes": [{
                "path": "/tmp/demo/src/App.tsx",
                "kind": { "type": "update", "move_path": null },
                "diff": "@@ -1 +1 @@\n-old\n+new\n"
            }]
        }),
        json!({
            "id": "mcp-1",
            "type": "mcpToolCall",
            "server": "docs",
            "tool": "search",
            "status": "completed",
            "arguments": { "query": "Codex" },
            "appContext": null,
            "pluginId": null,
            "result": { "content": [], "structuredContent": null, "_meta": null },
            "error": null,
            "durationMs": 42
        }),
        json!({
            "id": "dynamic-1",
            "type": "dynamicToolCall",
            "namespace": null,
            "tool": "render",
            "arguments": {},
            "status": "completed",
            "contentItems": [{ "type": "inputText", "text": "done" }],
            "success": true,
            "durationMs": 10
        }),
        json!({
            "id": "collab-1",
            "type": "collabAgentToolCall",
            "tool": "spawnAgent",
            "status": "completed",
            "senderThreadId": "thread-1",
            "receiverThreadIds": ["thread-2"],
            "prompt": "检查测试",
            "model": null,
            "reasoningEffort": null,
            "agentsStates": {
                "thread-2": { "status": "completed", "message": "完成" }
            }
        }),
        json!({
            "id": "activity-1",
            "type": "subAgentActivity",
            "kind": "interacted",
            "agentThreadId": "thread-2",
            "agentPath": "reviewer"
        }),
        json!({
            "id": "search-1",
            "type": "webSearch",
            "query": "Codex app server",
            "action": { "type": "openPage", "url": "https://developers.openai.com" }
        }),
        json!({ "id": "view-1", "type": "imageView", "path": "/tmp/demo/a.png" }),
        json!({
            "id": "image-1",
            "type": "imageGeneration",
            "status": "completed",
            "revisedPrompt": null,
            "result": "opaque-result",
            "savedPath": "/tmp/demo/generated.png"
        }),
        json!({ "id": "sleep-1", "type": "sleep", "durationMs": 100 }),
        json!({ "id": "review-in-1", "type": "enteredReviewMode", "review": "review" }),
        json!({ "id": "review-out-1", "type": "exitedReviewMode", "review": "review" }),
        json!({ "id": "compact-1", "type": "contextCompaction" }),
    ];
    assert_eq!(items.len(), 18);

    for item in items {
        let item_id = extract_string(&item, "/id").expect("fixture item id");
        let item_type = extract_string(&item, "/type").expect("fixture item type");
        let started = normalize_codex_notification_at(
            &json!({
                "method": "item/started",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "startedAtMs": 1783900800000_i64,
                    "item": item.clone()
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let completed = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800100_i64,
                    "item": item.clone()
                }
            }),
            "2026-07-13T00:00:01Z",
        );

        assert!(matches!(
            started.as_slice(),
            [AgentEvent::ItemStarted {
                item_id: actual_id,
                item_type: actual_type,
                item: actual_item,
                ..
            }] if actual_id == &item_id
                && actual_type == &item_type
                && actual_item.pointer("/id").and_then(Value::as_str) == Some(item_id.as_str())
                && actual_item.pointer("/type").and_then(Value::as_str) == Some(item_type.as_str())
        ));
        assert!(matches!(
            completed.as_slice(),
            [AgentEvent::ItemCompleted {
                item_id: actual_id,
                item_type: actual_type,
                item: actual_item,
                ..
            }] if actual_id == &item_id
                && actual_type == &item_type
                && actual_item.pointer("/id").and_then(Value::as_str) == Some(item_id.as_str())
                && actual_item.pointer("/type").and_then(Value::as_str) == Some(item_type.as_str())
        ));
    }
}

#[test]
fn lifecycle_ui_projection_omits_raw_binary_stdin_and_sensitive_tool_data() {
    let reasoning = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1783900800000_i64,
                "item": {
                    "id": "reasoning-1",
                    "type": "reasoning",
                    "summary": ["安全摘要"],
                    "content": ["RAW_REASONING_MUST_NOT_CROSS_IPC"]
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let image = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1783900800000_i64,
                "item": {
                    "id": "image-1",
                    "type": "imageGeneration",
                    "status": "completed",
                    "revisedPrompt": null,
                    "result": "data:image/png;base64,BASE64_IMAGE_MUST_NOT_CROSS_IPC",
                    "savedPath": "/tmp/demo/image.png"
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let mcp = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1783900800000_i64,
                "item": {
                    "id": "mcp-1",
                    "type": "mcpToolCall",
                    "server": "docs",
                    "tool": "read",
                    "status": "completed",
                    "arguments": {
                        "query": "safe query",
                        "apiKey": "MCP_SECRET_MUST_NOT_CROSS_IPC",
                        "url": "https://example.test/file?X-Amz-Signature=SIGNED_URL_MUST_NOT_CROSS_IPC"
                    },
                    "result": {
                        "content": [{
                            "type": "image",
                            "data": "data:image/png;base64,MCP_BASE64_MUST_NOT_CROSS_IPC"
                        }],
                        "structuredContent": null,
                        "_meta": { "cookie": "MCP_COOKIE_MUST_NOT_CROSS_IPC" }
                    },
                    "error": null,
                    "durationMs": 5
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let dynamic = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1783900800000_i64,
                "item": {
                    "id": "dynamic-1",
                    "type": "dynamicToolCall",
                    "namespace": null,
                    "tool": "render",
                    "arguments": { "password": "DYNAMIC_SECRET_MUST_NOT_CROSS_IPC" },
                    "status": "completed",
                    "contentItems": [{
                        "type": "inputImage",
                        "imageUrl": "data:image/png;base64,DYNAMIC_BASE64_MUST_NOT_CROSS_IPC"
                    }],
                    "success": true,
                    "durationMs": 5
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let command = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1783900800000_i64,
                "item": {
                    "id": "command-1",
                    "type": "commandExecution",
                    "command": "read -s password",
                    "cwd": "/tmp/demo",
                    "status": "completed",
                    "commandActions": [],
                    "stdin": "TERMINAL_SECRET_MUST_NOT_CROSS_IPC",
                    "aggregatedOutput": "x".repeat(AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS + 100),
                    "exitCode": 0,
                    "durationMs": 5
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let command_with_secret_output = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1783900800000_i64,
                "item": {
                    "id": "command-secret-output",
                    "type": "commandExecution",
                    "command": "printenv",
                    "cwd": "/tmp/demo",
                    "status": "completed",
                    "commandActions": [],
                    "aggregatedOutput": "token=COMMAND_OUTPUT_SECRET_MUST_NOT_CROSS_IPC",
                    "exitCode": 0,
                    "durationMs": 5
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let unknown = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1783900800000_i64,
                "item": {
                    "id": "future-1",
                    "type": "futureTool",
                    "safe": "visible",
                    "rawText": "UNKNOWN_RAW_MUST_NOT_CROSS_IPC",
                    "nested": { "authorization": "Bearer UNKNOWN_SECRET" }
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );

    let [AgentEvent::ItemCompleted { item, .. }] = reasoning.as_slice() else {
        panic!("reasoning lifecycle event");
    };
    assert!(item.get("content").is_none());
    assert_eq!(item.get("rawTextAvailable"), Some(&Value::Bool(true)));
    assert_eq!(item.get("contentCount").and_then(Value::as_u64), Some(1));

    let [AgentEvent::ItemCompleted { item, .. }] = image.as_slice() else {
        panic!("image lifecycle event");
    };
    assert!(item.get("result").is_none());
    assert_eq!(item.get("resultAvailable"), Some(&Value::Bool(true)));
    assert!(item.get("resultLength").and_then(Value::as_u64).is_some());

    let [AgentEvent::ItemCompleted { item, .. }] = command.as_slice() else {
        panic!("command lifecycle event");
    };
    assert!(item.get("stdin").is_none());
    assert_eq!(
        item.get("aggregatedOutput")
            .and_then(Value::as_str)
            .map(|output| output.chars().count()),
        Some(AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS)
    );
    assert_eq!(
        item.get("aggregatedOutputTruncated"),
        Some(&Value::Bool(true))
    );

    let [AgentEvent::ItemCompleted { item, .. }] = command_with_secret_output.as_slice() else {
        panic!("command secret output lifecycle event");
    };
    assert_eq!(
        item.get("aggregatedOutputTruncated"),
        Some(&Value::Bool(true))
    );

    let [AgentEvent::ItemCompleted { item, .. }] = mcp.as_slice() else {
        panic!("mcp lifecycle event");
    };
    assert_eq!(
        item.pointer("/arguments/_uiProjectionTruncated"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        item.pointer("/result/_uiProjectionTruncated"),
        Some(&Value::Bool(true))
    );

    let [AgentEvent::ItemCompleted { item, .. }] = dynamic.as_slice() else {
        panic!("dynamic lifecycle event");
    };
    assert!(item
        .get("contentItems")
        .and_then(Value::as_array)
        .and_then(|items| items.last())
        .and_then(|marker| marker.get("_uiProjectionTruncated"))
        .and_then(Value::as_bool)
        .unwrap_or(false));

    let [AgentEvent::ItemCompleted { item, .. }] = unknown.as_slice() else {
        panic!("unknown lifecycle event");
    };
    assert_eq!(item.get("_uiProjectionTruncated"), Some(&Value::Bool(true)));

    let serialized = serde_json::to_string(&[
        reasoning,
        image,
        mcp,
        dynamic,
        command,
        command_with_secret_output,
        unknown,
    ])
    .expect("projected lifecycle events should serialize");
    for forbidden in [
        "RAW_REASONING_MUST_NOT_CROSS_IPC",
        "BASE64_IMAGE_MUST_NOT_CROSS_IPC",
        "MCP_SECRET_MUST_NOT_CROSS_IPC",
        "SIGNED_URL_MUST_NOT_CROSS_IPC",
        "MCP_BASE64_MUST_NOT_CROSS_IPC",
        "MCP_COOKIE_MUST_NOT_CROSS_IPC",
        "DYNAMIC_SECRET_MUST_NOT_CROSS_IPC",
        "DYNAMIC_BASE64_MUST_NOT_CROSS_IPC",
        "TERMINAL_SECRET_MUST_NOT_CROSS_IPC",
        "COMMAND_OUTPUT_SECRET_MUST_NOT_CROSS_IPC",
        "UNKNOWN_RAW_MUST_NOT_CROSS_IPC",
        "UNKNOWN_SECRET",
    ] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }
    assert!(serialized.contains("safe query"));
    assert!(serialized.contains("visible"));
}

#[test]
fn status_text_projection_redacts_inline_credential_markers() {
    let warning = normalize_codex_notification_at(
        &json!({
            "method": "warning",
            "params": {
                "threadId": "thread-1",
                "message": "request failed: token=INLINE_TOKEN_MUST_NOT_CROSS_IPC"
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let error = normalize_codex_notification_at(
        &json!({
            "method": "error",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": true,
                "error": { "message": "Bearer INLINE_BEARER_MUST_NOT_CROSS_IPC" }
            }
        }),
        "2026-07-13T00:00:01Z",
    );
    let progress = normalize_codex_notification_at(
        &json!({
            "method": "item/mcpToolCall/progress",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "mcp-1",
                "message": "using sk-INLINE_KEY_MUST_NOT_CROSS_IPC"
            }
        }),
        "2026-07-13T00:00:02Z",
    );
    let hook = normalize_codex_notification_at(
        &json!({
            "method": "hook/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "run": {
                    "id": "hook-secret",
                    "eventName": "postToolUse",
                    "handlerType": "command",
                    "executionMode": "sync",
                    "scope": "turn",
                    "sourcePath": "/tmp/demo/.codex/hooks.json",
                    "source": "project",
                    "displayOrder": 0,
                    "status": "failed",
                    "statusMessage": "authorization: INLINE_AUTH_MUST_NOT_CROSS_IPC",
                    "startedAt": 1000,
                    "completedAt": 1200,
                    "durationMs": 200,
                    "entries": [{
                        "kind": "error",
                        "text": "-----BEGIN PRIVATE KEY-----\nINLINE_PRIVATE_KEY"
                    }]
                }
            }
        }),
        "2026-07-13T00:00:03Z",
    );

    let serialized = serde_json::to_string(&[warning, error, progress, hook])
        .expect("status events should serialize");
    for forbidden in [
        "INLINE_TOKEN_MUST_NOT_CROSS_IPC",
        "INLINE_BEARER_MUST_NOT_CROSS_IPC",
        "INLINE_KEY_MUST_NOT_CROSS_IPC",
        "INLINE_AUTH_MUST_NOT_CROSS_IPC",
        "INLINE_PRIVATE_KEY",
    ] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }
    assert!(serialized.contains("redacted credential or binary data"));
    assert_eq!(
        project_ui_sensitive_text("task-123 completed", AGENT_UI_STATUS_MESSAGE_CHARS),
        "task-123 completed"
    );
}

#[test]
fn structured_ui_projection_marks_field_array_node_and_character_limits() {
    let many_fields = Value::Object(
        (0..(AGENT_UI_STRUCTURED_MAX_FIELDS + 20))
            .map(|index| (format!("field-{index}"), json!(index)))
            .collect(),
    );
    let projected_fields = project_ui_safe_value(&many_fields);
    assert_eq!(
        projected_fields.get("_uiProjectionTruncated"),
        Some(&Value::Bool(true))
    );
    assert!(projected_fields.get("_omittedFieldCount").is_some());

    let many_items = json!((0..(AGENT_UI_STRUCTURED_MAX_ITEMS + 20)).collect::<Vec<_>>());
    let projected_items = project_ui_safe_value(&many_items);
    let projected_items = projected_items
        .as_array()
        .expect("array projection should stay an array");
    assert_eq!(projected_items.len(), AGENT_UI_STRUCTURED_MAX_ITEMS + 1);
    assert_eq!(
        projected_items
            .last()
            .and_then(|marker| marker.get("_uiProjectionTruncated")),
        Some(&Value::Bool(true))
    );

    let deep_value = json!({ "a": { "b": { "c": { "d": { "e": { "f": "deep" } } } } } });
    let projected_deep_value = project_ui_safe_value(&deep_value);
    assert_eq!(
        projected_deep_value.get("_uiProjectionTruncated"),
        Some(&Value::Bool(true))
    );

    let long_scalar = Value::String("long text ".repeat(AGENT_UI_TEXT_PREVIEW_CHARS));
    let projected_scalar = project_ui_safe_value(&long_scalar);
    assert_eq!(
        projected_scalar.get("_uiProjectionTruncated"),
        Some(&Value::Bool(true))
    );
    assert!(projected_scalar.get("preview").is_some());
}

#[test]
fn turn_and_plan_status_payloads_are_bounded() {
    let long_text = "z".repeat(AGENT_UI_LONG_TEXT_CHARS + 500);
    let final_events = normalize_codex_notification_at(
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "status": "completed",
                    "items": [{
                        "id": "message-1",
                        "type": "agentMessage",
                        "phase": "final_answer",
                        "text": long_text
                    }]
                }
            }
        }),
        "2026-07-13T00:00:00Z",
    );
    let plan_steps = (0..(AGENT_UI_PLAN_STEPS + 10))
        .map(|index| {
            json!({
                "step": format!("{index}:{}", "step ".repeat(400)),
                "status": "inProgress"
            })
        })
        .collect::<Vec<_>>();
    let plan_events = normalize_codex_notification_at(
        &json!({
            "method": "turn/plan/updated",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "explanation": "explanation ".repeat(1000),
                "plan": plan_steps
            }
        }),
        "2026-07-13T00:00:01Z",
    );

    let [AgentEvent::TurnCompleted {
        final_message: Some(final_message),
        ..
    }] = final_events.as_slice()
    else {
        panic!("turn completion event");
    };
    assert!(final_message.chars().count() <= AGENT_UI_LONG_TEXT_CHARS);

    let [AgentEvent::PlanUpdated {
        explanation: Some(explanation),
        plan,
        ..
    }] = plan_events.as_slice()
    else {
        panic!("plan update event");
    };
    assert!(explanation.chars().count() <= AGENT_UI_STATUS_MESSAGE_CHARS);
    assert_eq!(plan.len(), AGENT_UI_PLAN_STEPS);
    assert!(plan
        .iter()
        .all(|step| step.step.chars().count() <= AGENT_UI_TEXT_PREVIEW_CHARS));
}

#[test]
fn ignores_unknown_or_incomplete_notifications() {
    assert!(normalize_codex_notification_at(
        &json!({
            "method": "unknown",
            "params": {}
        }),
        "2026-06-24T00:00:00Z",
    )
    .is_empty());
    assert!(normalize_codex_notification_at(
        &json!({
            "method": "thread/started",
            "params": {}
        }),
        "2026-06-24T00:00:00Z",
    )
    .is_empty());
}
