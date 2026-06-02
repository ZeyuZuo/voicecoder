use super::{
    mock::MockAsrProvider, read_local_env, tencent::TencentAsrConfig, tencent::TencentAsrProvider,
    AsrProvider, VoiceProviderDiagnostic, VoiceProviderKind,
};

pub(crate) struct ProviderRegistry;

impl ProviderRegistry {
    pub(crate) fn provider_for_kind(
        provider: VoiceProviderKind,
    ) -> Result<Box<dyn AsrProvider + Send>, String> {
        match provider {
            VoiceProviderKind::Auto => {
                Err("auto provider must be resolved before session start".to_string())
            }
            VoiceProviderKind::Mock => Ok(Box::new(MockAsrProvider)),
            VoiceProviderKind::Tencent => Ok(Box::new(TencentAsrProvider)),
        }
    }

    pub(crate) fn diagnostics() -> Vec<VoiceProviderDiagnostic> {
        vec![
            MockAsrProvider.diagnostic(),
            TencentAsrProvider.diagnostic(),
        ]
    }

    pub(crate) fn resolve_provider(provider: VoiceProviderKind) -> VoiceProviderKind {
        match provider {
            VoiceProviderKind::Auto => Self::provider_override_from_env().unwrap_or_else(|| {
                if TencentAsrConfig::is_available() {
                    VoiceProviderKind::Tencent
                } else {
                    VoiceProviderKind::Mock
                }
            }),
            explicit_provider => explicit_provider,
        }
    }

    pub(crate) fn provider_override_from_env() -> Option<VoiceProviderKind> {
        read_local_env("VOICECODER_ASR_PROVIDER")
            .and_then(|value| Self::parse_provider_override(&value))
    }

    fn parse_provider_override(value: &str) -> Option<VoiceProviderKind> {
        match value.trim().to_lowercase().as_str() {
            "mock" => Some(VoiceProviderKind::Mock),
            "tencent" => Some(VoiceProviderKind::Tencent),
            "auto" => None,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_override_parser_accepts_known_values() {
        assert_eq!(
            ProviderRegistry::parse_provider_override("mock"),
            Some(VoiceProviderKind::Mock)
        );
        assert_eq!(
            ProviderRegistry::parse_provider_override(" Tencent "),
            Some(VoiceProviderKind::Tencent)
        );
        assert_eq!(ProviderRegistry::parse_provider_override("auto"), None);
        assert_eq!(ProviderRegistry::parse_provider_override("unknown"), None);
    }
}
