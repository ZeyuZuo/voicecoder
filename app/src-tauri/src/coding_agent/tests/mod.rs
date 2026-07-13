//! Coding-agent tests grouped by transport, protocol behavior, and UI projection.

pub(super) const THREAD_START_REQUEST_FIXTURE: &str =
    include_str!("../../../tests/fixtures/codex-app-server-v2/thread-start-request.json");
pub(super) const TURN_START_REQUEST_FIXTURE: &str =
    include_str!("../../../tests/fixtures/codex-app-server-v2/turn-start-request.json");
pub(super) const FILE_CHANGE_STARTED_FIXTURE: &str = include_str!(
    "../../../tests/fixtures/codex-app-server-v2/file-change-started-notification.json"
);
pub(super) const FILE_CHANGE_APPROVAL_REQUEST_FIXTURE: &str =
    include_str!("../../../tests/fixtures/codex-app-server-v2/file-change-approval-request.json");
pub(super) const COMMAND_APPROVAL_REQUEST_FIXTURE: &str =
    include_str!("../../../tests/fixtures/codex-app-server-v2/command-approval-request.json");
pub(super) const PERMISSIONS_APPROVAL_REQUEST_FIXTURE: &str =
    include_str!("../../../tests/fixtures/codex-app-server-v2/permissions-approval-request.json");
pub(super) const TOOL_USER_INPUT_REQUEST_FIXTURE: &str =
    include_str!("../../../tests/fixtures/codex-app-server-v2/tool-user-input-request.json");
pub(super) const MCP_ELICITATION_REQUEST_FIXTURE: &str =
    include_str!("../../../tests/fixtures/codex-app-server-v2/mcp-elicitation-request.json");
pub(super) const SERVER_REQUEST_RESOLVED_FIXTURE: &str = include_str!(
    "../../../tests/fixtures/codex-app-server-v2/server-request-resolved-notification.json"
);
pub(super) const AUTO_APPROVAL_COMPLETED_FIXTURE: &str = include_str!(
    "../../../tests/fixtures/codex-app-server-v2/auto-approval-completed-notification.json"
);

mod projection;
mod protocol;
mod transport;
