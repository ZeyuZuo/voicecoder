use super::super::*;

#[test]
fn normalizes_thread_and_turn_started_notifications() {
    let thread_events = normalize_codex_notification_at(
        &json!({
            "method": "thread/started",
            "params": {
                "thread": {
                    "id": "thread-1"
                }
            }
        }),
        "2026-06-24T00:00:00Z",
    );
    let turn_events = normalize_codex_notification_at(
        &json!({
            "method": "turn/started",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1"
                }
            }
        }),
        "2026-06-24T00:00:01Z",
    );

    assert_eq!(
        thread_events,
        vec![AgentEvent::ThreadStarted {
            thread_id: "thread-1".to_string(),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
    assert_eq!(
        turn_events,
        vec![AgentEvent::TurnStarted {
            turn_id: Some("turn-1".to_string()),
            created_at: "2026-06-24T00:00:01Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_agent_message_and_plan_deltas() {
    let message_events = normalize_codex_notification_at(
        &json!({
            "method": "item/agentMessage/delta",
            "params": {
                "delta": "正在修改首页",
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1"
            }
        }),
        "2026-06-24T00:00:00Z",
    );
    let plan_events = normalize_codex_notification_at(
        &json!({
            "method": "item/plan/delta",
            "params": {
                "delta": "实现主要布局",
                "itemId": "item-2",
                "threadId": "thread-1",
                "turnId": "turn-1"
            }
        }),
        "2026-06-24T00:00:01Z",
    );

    assert_eq!(
        message_events,
        vec![AgentEvent::ItemDelta {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "item-1".to_string(),
            item_type: "agentMessage".to_string(),
            lifecycle: "in_progress".to_string(),
            method: "item/agentMessage/delta".to_string(),
            delta: json!("正在修改首页"),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
    assert_eq!(
        plan_events,
        vec![AgentEvent::ItemDelta {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "item-2".to_string(),
            item_type: "plan".to_string(),
            lifecycle: "in_progress".to_string(),
            method: "item/plan/delta".to_string(),
            delta: json!("实现主要布局"),
            created_at: "2026-06-24T00:00:01Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_turn_plan_updated_notification() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "turn/plan/updated",
            "params": {
                "explanation": "计划已更新",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "plan": [
                    { "step": "读取项目结构", "status": "completed" },
                    { "step": "实现 demo", "status": "inProgress" }
                ]
            }
        }),
        "2026-06-24T00:00:00Z",
    );

    assert_eq!(
        events,
        vec![AgentEvent::PlanUpdated {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            explanation: Some("计划已更新".to_string()),
            plan: vec![
                AgentPlanStep {
                    step: "读取项目结构".to_string(),
                    status: "completed".to_string(),
                },
                AgentPlanStep {
                    step: "实现 demo".to_string(),
                    status: "inProgress".to_string(),
                },
            ],
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_command_execution_items() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "completedAtMs": 1782259200000_i64,
                "item": {
                    "id": "item-1",
                    "type": "commandExecution",
                    "command": "npm run check",
                    "commandActions": [],
                    "cwd": "/tmp/demo",
                    "status": "completed"
                },
                "threadId": "thread-1",
                "turnId": "turn-1"
            }
        }),
        "2026-06-24T00:00:00Z",
    );

    assert_eq!(
        events,
        vec![AgentEvent::ItemCompleted {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "item-1".to_string(),
            item_type: "commandExecution".to_string(),
            lifecycle: "completed".to_string(),
            status: Some("completed".to_string()),
            completed_at: "2026-06-24T00:00:00+00:00".to_string(),
            item: json!({
                "id": "item-1",
                "type": "commandExecution",
                "command": "npm run check",
                "commandActions": [],
                "cwd": "/tmp/demo",
                "status": "completed"
            }),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_command_execution_items_without_status_as_unknown() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1782259200000_i64,
                "item": {
                    "id": "item-1",
                    "type": "commandExecution",
                    "command": "npm run dev"
                }
            }
        }),
        "2026-06-24T00:00:00Z",
    );

    assert_eq!(
        events,
        vec![AgentEvent::ItemStarted {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "item-1".to_string(),
            item_type: "commandExecution".to_string(),
            lifecycle: "in_progress".to_string(),
            status: None,
            started_at: "2026-06-24T00:00:00+00:00".to_string(),
            item: json!({
                "id": "item-1",
                "type": "commandExecution",
                "command": "npm run dev"
            }),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_file_change_notifications() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "item/fileChange/patchUpdated",
            "params": {
                "changes": [
                    {
                        "path": "/tmp/demo/src/App.tsx",
                        "kind": { "type": "update" },
                        "diff": "@@"
                    },
                    {
                        "path": "/tmp/demo/src/styles.css",
                        "kind": { "type": "add" },
                        "diff": "@@"
                    }
                ],
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1"
            }
        }),
        "2026-06-24T00:00:00Z",
    );

    assert_eq!(
        events,
        vec![AgentEvent::ItemDelta {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "item-1".to_string(),
            item_type: "fileChange".to_string(),
            lifecycle: "in_progress".to_string(),
            method: "item/fileChange/patchUpdated".to_string(),
            delta: json!([
                {
                    "path": "/tmp/demo/src/App.tsx",
                    "kind": { "type": "update" },
                    "diff": "@@"
                },
                {
                    "path": "/tmp/demo/src/styles.css",
                    "kind": { "type": "add" },
                    "diff": "@@"
                }
            ]),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_command_output_delta_for_the_same_item() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "item/commandExecution/outputDelta",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "command-1",
                "delta": "tests passed\n"
            }
        }),
        "2026-07-13T00:00:00Z",
    );

    assert_eq!(
        events,
        vec![AgentEvent::ItemDelta {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "command-1".to_string(),
            item_type: "commandExecution".to_string(),
            lifecycle: "in_progress".to_string(),
            method: "item/commandExecution/outputDelta".to_string(),
            delta: json!("tests passed\n"),
            created_at: "2026-07-13T00:00:00Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_turn_diff_as_latest_snapshot() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "turn/diff/updated",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "diff": "diff --git a/src/App.tsx b/src/App.tsx\n-old\n+new\n"
            }
        }),
        "2026-07-13T00:00:00Z",
    );

    assert_eq!(
        events,
        vec![AgentEvent::TurnDiffUpdated {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            diff: "diff --git a/src/App.tsx b/src/App.tsx\n-old\n+new\n".to_string(),
            created_at: "2026-07-13T00:00:00Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_turn_completed_with_final_message_and_error() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "status": "failed",
                    "items": [
                        {
                            "id": "item-1",
                            "type": "agentMessage",
                            "text": "已完成第一版。"
                        }
                    ],
                    "error": {
                        "message": "Network blocked"
                    }
                }
            }
        }),
        "2026-06-24T00:00:00Z",
    );

    assert_eq!(
        events,
        vec![
            AgentEvent::TurnCompleted {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                status: "failed".to_string(),
                final_message: Some("已完成第一版。".to_string()),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            },
            AgentEvent::Error {
                message: "Network blocked".to_string(),
                retryable: false,
                terminal: true,
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            },
        ]
    );
}

#[test]
fn preserves_agent_message_phase_and_item_lifecycle_timestamps() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "completedAtMs": 1782259200000_i64,
                "item": {
                    "id": "message-1",
                    "type": "agentMessage",
                    "text": "最终答复",
                    "phase": "final_answer"
                }
            }
        }),
        "2026-06-24T00:00:01Z",
    );

    assert_eq!(
        events,
        vec![AgentEvent::ItemCompleted {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "message-1".to_string(),
            item_type: "agentMessage".to_string(),
            lifecycle: "completed".to_string(),
            status: None,
            completed_at: "2026-06-24T00:00:00+00:00".to_string(),
            item: json!({
                "id": "message-1",
                "type": "agentMessage",
                "text": "最终答复",
                "phase": "final_answer"
            }),
            created_at: "2026-06-24T00:00:01Z".to_string(),
        }]
    );
}

#[test]
fn distinguishes_warnings_retryable_errors_and_terminal_errors() {
    let warning = normalize_codex_notification_at(
        &json!({
            "method": "warning",
            "params": {
                "message": "上下文接近限制",
                "threadId": "thread-1"
            }
        }),
        "2026-06-24T00:00:00Z",
    );
    let retryable = normalize_codex_notification_at(
        &json!({
            "method": "error",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": true,
                "error": { "message": "连接中断" }
            }
        }),
        "2026-06-24T00:00:01Z",
    );
    let terminal = normalize_codex_notification_at(
        &json!({
            "method": "error",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "willRetry": false,
                "error": { "message": "重试次数耗尽" }
            }
        }),
        "2026-06-24T00:00:02Z",
    );

    assert!(matches!(warning.as_slice(), [AgentEvent::Warning { .. }]));
    assert!(matches!(
        retryable.as_slice(),
        [AgentEvent::Error {
            retryable: true,
            terminal: false,
            ..
        }]
    ));
    assert!(matches!(
        terminal.as_slice(),
        [AgentEvent::Error {
            retryable: false,
            terminal: true,
            ..
        }]
    ));

    let mut summary = AgentRunEventSummary::default();
    update_agent_run_summary(&mut summary, &retryable[0]);
    assert!(!summary.terminal);
    assert!(summary.error_message.is_none());
    update_agent_run_summary(&mut summary, &terminal[0]);
    assert!(summary.terminal);
    assert_eq!(summary.completion_status(), "failed");
}

#[test]
fn preserves_interrupted_turn_status() {
    let events = normalize_codex_notification_at(
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "status": "interrupted",
                    "items": []
                }
            }
        }),
        "2026-06-24T00:00:00Z",
    );
    let mut summary = AgentRunEventSummary::default();
    update_agent_run_summary(&mut summary, &events[0]);

    assert!(matches!(
        events.as_slice(),
        [AgentEvent::TurnCompleted { status, .. }] if status == "interrupted"
    ));
    assert!(summary.terminal);
    assert_eq!(summary.completion_status(), "interrupted");
}

#[test]
fn normalizes_codex_exec_json_lifecycle_events() {
    let thread_events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "thread.started",
            "thread_id": "exec-thread-1"
        }),
        "2026-06-24T00:00:00Z",
    );
    let turn_events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "turn.started"
        }),
        "2026-06-24T00:00:01Z",
    );
    let completed_events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "turn.completed",
            "final_message": "完成"
        }),
        "2026-06-24T00:00:02Z",
    );

    assert_eq!(
        thread_events,
        vec![AgentEvent::ThreadStarted {
            thread_id: "exec-thread-1".to_string(),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
    assert_eq!(
        turn_events,
        vec![AgentEvent::TurnStarted {
            turn_id: None,
            created_at: "2026-06-24T00:00:01Z".to_string(),
        }]
    );
    assert_eq!(
        completed_events,
        vec![AgentEvent::TurnCompleted {
            thread_id: None,
            turn_id: None,
            status: "completed".to_string(),
            final_message: Some("完成".to_string()),
            created_at: "2026-06-24T00:00:02Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_codex_exec_json_item_events() {
    let command_events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "item.started",
            "item": {
                "id": "item-1",
                "type": "command_execution",
                "command": "npm run check",
                "status": "in_progress"
            }
        }),
        "2026-06-24T00:00:00Z",
    );
    let message_events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "item.completed",
            "item": {
                "id": "item-2",
                "type": "agent_message",
                "text": "Repo contains docs, sdk, and examples directories."
            }
        }),
        "2026-06-24T00:00:01Z",
    );
    let file_events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "item.completed",
            "item": {
                "id": "item-3",
                "type": "file_change",
                "changes": [
                    {
                        "path": "src/App.tsx",
                        "kind": { "type": "update" }
                    }
                ]
            }
        }),
        "2026-06-24T00:00:02Z",
    );

    assert_eq!(
        command_events,
        vec![AgentEvent::Command {
            command: "npm run check".to_string(),
            status: "in_progress".to_string(),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
    assert_eq!(
        message_events,
        vec![AgentEvent::AgentMessage {
            text: "Repo contains docs, sdk, and examples directories.".to_string(),
            created_at: "2026-06-24T00:00:01Z".to_string(),
        }]
    );
    assert_eq!(
        file_events,
        vec![AgentEvent::FileChange {
            path: "src/App.tsx".to_string(),
            change_type: Some("update".to_string()),
            created_at: "2026-06-24T00:00:02Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_codex_exec_json_single_file_change_with_camel_case_change_type() {
    let events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "item.completed",
            "item": {
                "id": "item-3",
                "type": "file_change",
                "path": "src/App.tsx",
                "changeType": "update"
            }
        }),
        "2026-06-24T00:00:02Z",
    );

    assert_eq!(
        events,
        vec![AgentEvent::FileChange {
            path: "src/App.tsx".to_string(),
            change_type: Some("update".to_string()),
            created_at: "2026-06-24T00:00:02Z".to_string(),
        }]
    );
}

#[test]
fn normalizes_codex_exec_json_failed_turn_and_errors() {
    let failed_events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "turn.failed",
            "error": {
                "message": "Sandbox denied command"
            }
        }),
        "2026-06-24T00:00:00Z",
    );
    let error_events = normalize_codex_exec_json_event_at(
        &json!({
            "type": "error",
            "message": "Authentication required"
        }),
        "2026-06-24T00:00:01Z",
    );

    assert_eq!(
        failed_events,
        vec![AgentEvent::Error {
            message: "Sandbox denied command".to_string(),
            retryable: false,
            terminal: true,
            thread_id: None,
            turn_id: None,
            created_at: "2026-06-24T00:00:00Z".to_string(),
        }]
    );
    assert_eq!(
        error_events,
        vec![AgentEvent::Error {
            message: "Authentication required".to_string(),
            retryable: false,
            terminal: true,
            thread_id: None,
            turn_id: None,
            created_at: "2026-06-24T00:00:01Z".to_string(),
        }]
    );
}

#[test]
fn serializes_agent_event_for_frontend_contract() {
    let value = serde_json::to_value(AgentEvent::FileChange {
        path: "/tmp/demo/src/App.tsx".to_string(),
        change_type: Some("update".to_string()),
        created_at: "2026-06-24T00:00:00Z".to_string(),
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "type": "file_change",
            "path": "/tmp/demo/src/App.tsx",
            "changeType": "update",
            "createdAt": "2026-06-24T00:00:00Z"
        })
    );
}

#[test]
fn serializes_agent_event_envelope_for_tauri_event_contract() {
    let value = serde_json::to_value(AgentEventEnvelope {
        demo_session_id: "demo-1".to_string(),
        run_id: "run-1".to_string(),
        event: AgentEvent::AgentMessage {
            text: "正在生成 demo".to_string(),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        },
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "demoSessionId": "demo-1",
            "runId": "run-1",
            "event": {
                "type": "agent_message",
                "text": "正在生成 demo",
                "createdAt": "2026-06-24T00:00:00Z"
            }
        })
    );
}

#[test]
fn serializes_agent_run_started_runtime_for_session_logs() {
    let value = serde_json::to_value(AgentRunStartedEvent {
        demo_session_id: "demo-1".to_string(),
        run_id: "run-1".to_string(),
        project_path: "/tmp/demo".to_string(),
        provider: CodingAgentProviderKind::CodexAppServer,
        codex_thread_id: "thread-1".to_string(),
        codex_turn_id: "turn-1".to_string(),
        runtime: CodingAgentRuntimeMetadata {
            provider: CodingAgentProviderKind::CodexAppServer,
            version: "codex-cli 0.144.1".to_string(),
            transport: "stdio".to_string(),
            sandbox: "workspace-write".to_string(),
            approval_policy: Some("on-request".to_string()),
            approvals_reviewer: Some("auto_review".to_string()),
            transport_log_path: Some(
                "/tmp/demo/.voicecoder/agent_run_run-1_app_server.jsonl".to_string(),
            ),
            protocol_baseline_version: CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION.to_string(),
            protocol_compatibility: "verified".to_string(),
        },
        started_at: "2026-07-13T00:00:00Z".to_string(),
    })
    .unwrap();

    assert_eq!(
        value.pointer("/runtime/version").and_then(Value::as_str),
        Some("codex-cli 0.144.1")
    );
    assert_eq!(
        value
            .pointer("/runtime/approvalPolicy")
            .and_then(Value::as_str),
        Some("on-request")
    );
    assert_eq!(
        value
            .pointer("/runtime/approvalsReviewer")
            .and_then(Value::as_str),
        Some("auto_review")
    );
    assert_eq!(
        value
            .pointer("/runtime/protocolBaselineVersion")
            .and_then(Value::as_str),
        Some(CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION)
    );
    assert_eq!(
        value
            .pointer("/runtime/protocolCompatibility")
            .and_then(Value::as_str),
        Some("verified")
    );
    assert_eq!(
        value
            .pointer("/runtime/transportLogPath")
            .and_then(Value::as_str),
        Some("/tmp/demo/.voicecoder/agent_run_run-1_app_server.jsonl")
    );
}

#[test]
fn serializes_agent_run_completed_event_for_tauri_event_contract() {
    let value = serde_json::to_value(AgentRunCompletedEvent {
        demo_session_id: "demo-1".to_string(),
        run_id: "run-1".to_string(),
        final_message: Some("完成".to_string()),
        changed_files: vec!["/tmp/demo/src/App.tsx".to_string()],
        status: "completed".to_string(),
        error: None,
        completed_at: "2026-06-24T00:00:00Z".to_string(),
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "demoSessionId": "demo-1",
            "runId": "run-1",
            "finalMessage": "完成",
            "changedFiles": ["/tmp/demo/src/App.tsx"],
            "status": "completed",
            "error": null,
            "completedAt": "2026-06-24T00:00:00Z"
        })
    );
}

#[test]
fn run_summary_collects_changed_files_final_message_and_errors() {
    let mut summary = AgentRunEventSummary::default();
    update_agent_run_summary(
        &mut summary,
        &AgentEvent::FileChange {
            path: "/tmp/demo/src/App.tsx".to_string(),
            change_type: Some("update".to_string()),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        },
    );
    update_agent_run_summary(
        &mut summary,
        &AgentEvent::FileChange {
            path: "/tmp/demo/src/App.tsx".to_string(),
            change_type: Some("update".to_string()),
            created_at: "2026-06-24T00:00:01Z".to_string(),
        },
    );
    update_agent_run_summary(
        &mut summary,
        &AgentEvent::TurnCompleted {
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            status: "completed".to_string(),
            final_message: Some("完成".to_string()),
            created_at: "2026-06-24T00:00:02Z".to_string(),
        },
    );

    assert_eq!(summary.changed_files, vec!["/tmp/demo/src/App.tsx"]);
    assert_eq!(summary.final_message, Some("完成".to_string()));
    assert!(summary.terminal);

    update_agent_run_summary(
        &mut summary,
        &AgentEvent::Error {
            message: "失败".to_string(),
            retryable: false,
            terminal: true,
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            created_at: "2026-06-24T00:00:03Z".to_string(),
        },
    );
    assert_eq!(summary.error_message, Some("失败".to_string()));
}
