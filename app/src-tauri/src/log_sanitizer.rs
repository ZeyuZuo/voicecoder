//! Shared redaction for diagnostic files that must never persist credentials.

use serde_json::{Map, Value};

pub(crate) const REDACTED_LOG_VALUE: &str = "[REDACTED_CREDENTIAL]";
const REDACTED_LOG_TEXT: &str = "[REDACTED_CREDENTIAL_TEXT]";

pub(crate) fn sanitize_json_for_log(value: &Value) -> Value {
    sanitize_json_value(value, None)
}

fn sanitize_json_value(value: &Value, key: Option<&str>) -> Value {
    if key.is_some_and(is_sensitive_log_key) {
        return Value::String(REDACTED_LOG_VALUE.to_string());
    }

    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), sanitize_json_value(value, Some(key))))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| sanitize_json_value(value, None))
                .collect(),
        ),
        Value::String(value) => Value::String(redact_log_text(value)),
        _ => value.clone(),
    }
}

fn is_sensitive_log_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();

    matches!(
        normalized.as_str(),
        "authorization"
            | "proxyauthorization"
            | "apikey"
            | "openaiapikey"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "authtoken"
            | "bearertoken"
            | "token"
            | "password"
            | "passphrase"
            | "secret"
            | "clientsecret"
            | "credential"
            | "credentials"
            | "cookie"
            | "setcookie"
            | "privatekey"
            | "signature"
            | "stdin"
            | "chatgptauthtokens"
    ) || normalized.ends_with("apikey")
        || normalized.ends_with("accesstoken")
        || normalized.ends_with("refreshtoken")
        || normalized.ends_with("clientsecret")
        || normalized.ends_with("privatekey")
}

fn redact_log_text(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let contains_assignment = lower.contains('=') || lower.contains(':');
    let credential_assignment = contains_assignment
        && [
            "authorization",
            "api_key",
            "api-key",
            "apikey",
            "access_token",
            "access-token",
            "accesstoken",
            "refresh_token",
            "refresh-token",
            "refreshtoken",
            "auth_token",
            "auth-token",
            "password",
            "passphrase",
            "client_secret",
            "client-secret",
            "private_key",
            "private-key",
            "credential",
            "cookie",
            "signature",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
    let bearer_value = lower.contains("bearer ");
    let private_key = lower.contains("-----begin") && lower.contains("private key-----");
    let openai_key = lower.match_indices("sk-").any(|(index, _)| {
        value[index..]
            .chars()
            .take_while(is_credential_char)
            .count()
            >= 12
    });

    if credential_assignment || bearer_value || private_key || openai_key {
        REDACTED_LOG_TEXT.to_string()
    } else {
        value.to_string()
    }
}

fn is_credential_char(character: &char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/' | '+' | '=')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_nested_credential_fields_without_hiding_token_usage() {
        let sanitized = sanitize_json_for_log(&json!({
            "authorization": "Bearer top-secret",
            "config": {
                "openaiApiKey": "sk-example-secret-value",
                "tokenUsage": { "totalTokens": 42 }
            },
            "items": [{ "refresh_token": "refresh-secret" }]
        }));

        assert_eq!(sanitized["authorization"], REDACTED_LOG_VALUE);
        assert_eq!(sanitized["config"]["openaiApiKey"], REDACTED_LOG_VALUE);
        assert_eq!(sanitized["items"][0]["refresh_token"], REDACTED_LOG_VALUE);
        assert_eq!(sanitized["config"]["tokenUsage"]["totalTokens"], 42);
    }

    #[test]
    fn redacts_credentials_embedded_in_stderr_or_invalid_json_text() {
        for value in [
            "Authorization: Bearer abcdefghijklmnop",
            "OPENAI_API_KEY=sk-example-secret-value",
            "-----BEGIN PRIVATE KEY----- abc",
        ] {
            assert_eq!(
                sanitize_json_for_log(&Value::String(value.to_string())),
                Value::String(REDACTED_LOG_TEXT.to_string())
            );
        }
    }

    #[test]
    fn keeps_normal_protocol_text_available_for_diagnostics() {
        assert_eq!(
            sanitize_json_for_log(&json!({
                "method": "thread/tokenUsage/updated",
                "message": "waiting for Codex"
            })),
            json!({
                "method": "thread/tokenUsage/updated",
                "message": "waiting for Codex"
            })
        );
    }
}
