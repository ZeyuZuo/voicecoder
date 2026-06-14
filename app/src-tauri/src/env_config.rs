use std::{env, fs, path::PathBuf};

pub(crate) fn read_local_env(key: &str) -> Option<String> {
    if let Ok(value) = env::var(key) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }

    for env_path in candidate_env_files() {
        let Ok(content) = fs::read_to_string(env_path) else {
            continue;
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let Some((line_key, line_value)) = trimmed.split_once('=') else {
                continue;
            };

            if line_key.trim() == key {
                let value = clean_env_value(line_value);
                if !value.trim().is_empty() {
                    return Some(value);
                }
            }
        }
    }

    None
}

fn candidate_env_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(current_dir) = env::current_dir() {
        paths.push(current_dir.join(".env"));
        paths.push(current_dir.join("app").join(".env"));

        if let Some(parent) = current_dir.parent() {
            paths.push(parent.join(".env"));
            paths.push(parent.join("app").join(".env"));
        }
    }

    paths
}

fn clean_env_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_quoted_env_values() {
        assert_eq!(clean_env_value(" value "), "value");
        assert_eq!(clean_env_value("\"quoted\""), "quoted");
        assert_eq!(clean_env_value("'quoted'"), "quoted");
    }
}
