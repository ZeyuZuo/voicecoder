use crate::env_config::read_local_env;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, process::Command};

const DEFAULT_CODEX_BIN: &str = "codex";

#[allow(dead_code)]
pub(crate) trait CodingAgentSession {
    fn cancel(&mut self) -> Result<(), String>;
}

#[allow(dead_code)]
pub(crate) trait CodingAgentProvider {
    fn kind(&self) -> CodingAgentProviderKind;
    fn validate_start(&self) -> Result<(), String>;
    fn diagnostic(&self) -> CodingAgentProviderDiagnostic;
    fn start_session(
        &self,
        _context: CodingAgentStartContext,
    ) -> Result<Box<dyn CodingAgentSession + Send>, String> {
        Err("Coding Agent session start is not implemented yet.".to_string())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentProviderKind {
    Auto,
    CodexAppServer,
    CodexExecJson,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentProviderStatus {
    auto_provider: CodingAgentProviderKind,
    provider_override: Option<CodingAgentProviderKind>,
    active_provider_configured: bool,
    active_provider_error: Option<String>,
    diagnostics: Vec<CodingAgentProviderDiagnostic>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentProviderDiagnostic {
    provider: CodingAgentProviderKind,
    configured: bool,
    missing_dependencies: Vec<String>,
    executable: Option<String>,
    version: Option<String>,
    details: BTreeMap<String, String>,
    error: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct CodingAgentStartContext {
    pub project_path: String,
    pub prompt: String,
}

pub(crate) struct CodingAgentProviderRegistry;

impl CodingAgentProviderRegistry {
    pub(crate) fn provider_for_kind(
        provider: CodingAgentProviderKind,
    ) -> Result<Box<dyn CodingAgentProvider + Send>, String> {
        match provider {
            CodingAgentProviderKind::Auto => {
                Err("auto coding agent provider must be resolved before use".to_string())
            }
            CodingAgentProviderKind::CodexAppServer => Ok(Box::new(CodexAppServerProvider)),
            CodingAgentProviderKind::CodexExecJson => Ok(Box::new(CodexExecJsonProvider)),
        }
    }

    pub(crate) fn diagnostics() -> Vec<CodingAgentProviderDiagnostic> {
        vec![
            CodexAppServerProvider.diagnostic(),
            CodexExecJsonProvider.diagnostic(),
        ]
    }

    pub(crate) fn resolve_provider(provider: CodingAgentProviderKind) -> CodingAgentProviderKind {
        match provider {
            CodingAgentProviderKind::Auto => Self::provider_override_from_env()
                .unwrap_or(CodingAgentProviderKind::CodexAppServer),
            explicit_provider => explicit_provider,
        }
    }

    pub(crate) fn provider_override_from_env() -> Option<CodingAgentProviderKind> {
        read_local_env("VOICECODER_CODING_AGENT_PROVIDER")
            .and_then(|value| Self::parse_provider_override(&value))
    }

    fn parse_provider_override(value: &str) -> Option<CodingAgentProviderKind> {
        match value.trim().to_lowercase().as_str() {
            "codex_app_server" => Some(CodingAgentProviderKind::CodexAppServer),
            "codex_exec_json" => Some(CodingAgentProviderKind::CodexExecJson),
            "auto" => None,
            _ => None,
        }
    }
}

pub(crate) struct CodexAppServerProvider;

impl CodingAgentProvider for CodexAppServerProvider {
    fn kind(&self) -> CodingAgentProviderKind {
        CodingAgentProviderKind::CodexAppServer
    }

    fn validate_start(&self) -> Result<(), String> {
        validate_codex_executable().map(|_| ())
    }

    fn diagnostic(&self) -> CodingAgentProviderDiagnostic {
        codex_diagnostic(
            self.kind(),
            [
                ("transport", "stdio"),
                ("command", "codex app-server --stdio"),
                ("threadMode", "persistent"),
            ],
        )
    }
}

pub(crate) struct CodexExecJsonProvider;

impl CodingAgentProvider for CodexExecJsonProvider {
    fn kind(&self) -> CodingAgentProviderKind {
        CodingAgentProviderKind::CodexExecJson
    }

    fn validate_start(&self) -> Result<(), String> {
        validate_codex_executable().map(|_| ())
    }

    fn diagnostic(&self) -> CodingAgentProviderDiagnostic {
        codex_diagnostic(
            self.kind(),
            [
                ("transport", "process-jsonl"),
                (
                    "command",
                    "codex exec --json --sandbox workspace-write --cd <project> <prompt>",
                ),
                ("threadMode", "single-run"),
            ],
        )
    }
}

#[tauri::command]
pub fn get_coding_agent_provider_status() -> CodingAgentProviderStatus {
    let provider_override = CodingAgentProviderRegistry::provider_override_from_env();
    let auto_provider =
        CodingAgentProviderRegistry::resolve_provider(CodingAgentProviderKind::Auto);
    let active_provider_error = CodingAgentProviderRegistry::provider_for_kind(auto_provider)
        .and_then(|provider| provider.validate_start())
        .err();

    CodingAgentProviderStatus {
        auto_provider,
        provider_override,
        active_provider_configured: active_provider_error.is_none(),
        active_provider_error,
        diagnostics: CodingAgentProviderRegistry::diagnostics(),
    }
}

fn codex_diagnostic<const N: usize>(
    provider: CodingAgentProviderKind,
    details: [(&str, &str); N],
) -> CodingAgentProviderDiagnostic {
    let executable = codex_executable();
    let version_result = validate_codex_executable();
    let version = version_result.clone().ok();
    let missing_dependencies = if version_result.is_ok() {
        Vec::new()
    } else {
        vec![executable.clone()]
    };
    let mut detail_map = details
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<BTreeMap<_, _>>();
    detail_map.insert(
        "executableSource".to_string(),
        if read_local_env("VOICECODER_CODEX_BIN").is_some() {
            "VOICECODER_CODEX_BIN".to_string()
        } else {
            "PATH".to_string()
        },
    );

    CodingAgentProviderDiagnostic {
        provider,
        configured: version_result.is_ok(),
        missing_dependencies,
        executable: Some(executable),
        version,
        details: detail_map,
        error: version_result.err(),
    }
}

fn validate_codex_executable() -> Result<String, String> {
    let executable = codex_executable();
    let output = Command::new(&executable)
        .arg("--version")
        .output()
        .map_err(|error| format!("无法执行 Codex CLI `{executable}`：{error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Codex CLI `{executable}` 返回非零退出码。")
        } else {
            format!("Codex CLI `{executable}` 检查失败：{stderr}")
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Err(format!("Codex CLI `{executable}` 没有返回版本信息。"));
    }

    Ok(stdout)
}

fn codex_executable() -> String {
    read_local_env("VOICECODER_CODEX_BIN").unwrap_or_else(|| DEFAULT_CODEX_BIN.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_override_parser_accepts_known_values() {
        assert_eq!(
            CodingAgentProviderRegistry::parse_provider_override("codex_app_server"),
            Some(CodingAgentProviderKind::CodexAppServer)
        );
        assert_eq!(
            CodingAgentProviderRegistry::parse_provider_override(" codex_exec_json "),
            Some(CodingAgentProviderKind::CodexExecJson)
        );
        assert_eq!(
            CodingAgentProviderRegistry::parse_provider_override("auto"),
            None
        );
        assert_eq!(
            CodingAgentProviderRegistry::parse_provider_override("unknown"),
            None
        );
    }

    #[test]
    fn auto_provider_resolves_to_app_server_by_default() {
        assert_eq!(
            CodingAgentProviderRegistry::resolve_provider(CodingAgentProviderKind::Auto),
            CodingAgentProviderKind::CodexAppServer
        );
    }
}
