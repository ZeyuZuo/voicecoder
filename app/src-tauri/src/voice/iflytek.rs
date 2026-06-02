use super::{
    read_local_env, AsrProvider, AsrSession, AsrStartContext, VoiceProviderDiagnostic,
    VoiceProviderKind,
};
use std::collections::BTreeMap;

const DEFAULT_IFLYTEK_LLM_ENDPOINT: &str =
    "wss://office-api-ast-dx.iflyaisol.com/ast/communicate/v1";
const DEFAULT_IFLYTEK_LLM_ROLE_TYPE: &str = "2";

pub(crate) struct IflytekLlmAsrProvider;

pub(crate) struct IflytekLlmConfig {
    app_id: String,
    access_key_id: String,
    endpoint: String,
    role_type: String,
    feature_ids: Option<String>,
}

impl AsrProvider for IflytekLlmAsrProvider {
    fn kind(&self) -> VoiceProviderKind {
        VoiceProviderKind::IflytekLlm
    }

    fn validate_start(&self) -> Result<(), String> {
        IflytekLlmConfig::from_env().map(|_| ())
    }

    fn diagnostic(&self) -> VoiceProviderDiagnostic {
        let missing_env = IflytekLlmConfig::missing_required_env();
        if !missing_env.is_empty() {
            return VoiceProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: Some(DEFAULT_IFLYTEK_LLM_ENDPOINT.to_string()),
                details: BTreeMap::new(),
                error: Some("讯飞大模型 ASR 凭证未配置完整。".to_string()),
            };
        }

        match IflytekLlmConfig::from_env() {
            Ok(config) => {
                let mut details = BTreeMap::new();
                details.insert("appId".to_string(), config.app_id);
                details.insert("accessKeyId".to_string(), config.access_key_id);
                details.insert("roleType".to_string(), config.role_type);
                details.insert(
                    "featureIdsCount".to_string(),
                    config
                        .feature_ids
                        .as_deref()
                        .map(count_feature_ids)
                        .unwrap_or(0)
                        .to_string(),
                );

                VoiceProviderDiagnostic {
                    provider: self.kind(),
                    configured: true,
                    missing_env,
                    endpoint: Some(config.endpoint),
                    details,
                    error: None,
                }
            }
            Err(error) => VoiceProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: Some(DEFAULT_IFLYTEK_LLM_ENDPOINT.to_string()),
                details: BTreeMap::new(),
                error: Some(error),
            },
        }
    }

    fn start_session(
        &self,
        _context: AsrStartContext,
    ) -> Result<Box<dyn AsrSession + Send>, String> {
        Err("讯飞大模型 ASR provider 已完成配置诊断，WebSocket 连接将在 Step 7 实现。".to_string())
    }
}

impl IflytekLlmConfig {
    pub(crate) fn from_env() -> Result<Self, String> {
        let _access_key_secret = required_env("IFLYTEK_LLM_ACCESS_KEY_SECRET")?;

        Ok(Self {
            app_id: required_env("IFLYTEK_LLM_APP_ID")?,
            access_key_id: required_env("IFLYTEK_LLM_ACCESS_KEY_ID")?,
            endpoint: read_local_env("IFLYTEK_LLM_ENDPOINT")
                .unwrap_or_else(|| DEFAULT_IFLYTEK_LLM_ENDPOINT.to_string()),
            role_type: read_local_env("IFLYTEK_LLM_ROLE_TYPE")
                .unwrap_or_else(|| DEFAULT_IFLYTEK_LLM_ROLE_TYPE.to_string()),
            feature_ids: read_local_env("IFLYTEK_LLM_FEATURE_IDS"),
        })
    }

    pub(crate) fn missing_required_env() -> Vec<String> {
        [
            "IFLYTEK_LLM_APP_ID",
            "IFLYTEK_LLM_ACCESS_KEY_ID",
            "IFLYTEK_LLM_ACCESS_KEY_SECRET",
        ]
        .iter()
        .filter(|key| required_env(key).is_err())
        .map(|key| (*key).to_string())
        .collect()
    }
}

fn required_env(key: &str) -> Result<String, String> {
    read_local_env(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("缺少本地环境变量 {key}，请先配置讯飞大模型 ASR 凭证。"))
}

fn count_feature_ids(value: &str) -> usize {
    value
        .split(',')
        .filter(|feature_id| !feature_id.trim().is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_configured_feature_ids() {
        assert_eq!(count_feature_ids(""), 0);
        assert_eq!(count_feature_ids("feature-a"), 1);
        assert_eq!(count_feature_ids("feature-a, feature-b,,feature-c"), 3);
    }
}
