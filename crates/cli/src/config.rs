//! Settings files, read as agent-browser reads its own: `--config` or
//! `MOBILE_AGENT_CONFIG` alone when given, else `~/.mobile-agent/config.json`
//! overlaid by `./mobile-agent.json`. Environment variables override the
//! files and flags override both. Keys are camelCase; unknown keys are
//! ignored.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// What a settings file can set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub json: Option<bool>,
    pub session: Option<String>,
    pub platform: Option<String>,
    pub device: Option<String>,
    /// The idle timeout of a daemon the CLI starts, as `--idle-timeout`
    /// takes it; a number is milliseconds.
    pub idle_timeout: Option<String>,
}

impl Config {
    fn from_json(value: &Value) -> Config {
        Config {
            json: value.get("json").and_then(Value::as_bool),
            session: value
                .get("session")
                .and_then(Value::as_str)
                .map(str::to_owned),
            platform: value
                .get("platform")
                .and_then(Value::as_str)
                .map(str::to_owned),
            device: value
                .get("device")
                .and_then(Value::as_str)
                .map(str::to_owned),
            idle_timeout: match value.get("idleTimeout") {
                Some(Value::String(text)) => Some(text.clone()),
                Some(Value::Number(ms)) => Some(ms.to_string()),
                _ => None,
            },
        }
    }

    /// This config with `top`'s settings in place of its own.
    fn overlay(self, top: Config) -> Config {
        Config {
            json: top.json.or(self.json),
            session: top.session.or(self.session),
            platform: top.platform.or(self.platform),
            device: top.device.or(self.device),
            idle_timeout: top.idle_timeout.or(self.idle_timeout),
        }
    }
}

fn read(path: &Path) -> Result<Option<Config>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;
    if !value.is_object() {
        return Err(format!("{} must hold a JSON object", path.display()));
    }
    Ok(Some(Config::from_json(&value)))
}

/// The user's config file.
fn user_file() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| {
            PathBuf::from(home)
                .join(".mobile-agent")
                .join("config.json")
        })
}

/// Loads the settings: `explicit` alone when given, which must exist; else
/// the user file overlaid by the project file in the working directory.
pub fn load(explicit: Option<&Path>) -> Result<Config, String> {
    if let Some(path) = explicit {
        return read(path)?.ok_or_else(|| format!("config file not found: {}", path.display()));
    }
    let user = match user_file() {
        Some(path) => read(&path)?.unwrap_or_default(),
        None => Config::default(),
    };
    let project = read(Path::new("mobile-agent.json"))?.unwrap_or_default();
    Ok(user.overlay(project))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_project_file_overrides_the_user_file_key_by_key() {
        let user =
            Config::from_json(&json!({"json": true, "session": "user", "idleTimeout": 5000}));
        let project =
            Config::from_json(&json!({"session": "project", "device": "FAKE-IOS-1", "extra": 1}));
        assert_eq!(
            user.overlay(project),
            Config {
                json: Some(true),
                session: Some("project".into()),
                platform: None,
                device: Some("FAKE-IOS-1".into()),
                idle_timeout: Some("5000".into()),
            }
        );
    }

    #[test]
    fn an_explicit_file_must_exist() {
        let missing = Path::new("/definitely/not/here/mobile-agent.json");
        assert!(load(Some(missing)).unwrap_err().contains("not found"));
    }
}
