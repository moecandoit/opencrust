use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static GOOGLE_RUNTIME_SECRETS: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();

fn runtime_secrets() -> &'static RwLock<HashMap<String, String>> {
    GOOGLE_RUNTIME_SECRETS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn set_runtime_secret(key: &str, value: &str) {
    let mut guard = runtime_secrets().write().unwrap_or_else(|e| e.into_inner());
    guard.insert(key.to_string(), value.to_string());
}

pub fn get_runtime_secret(key: &str) -> Option<String> {
    let guard = runtime_secrets().read().unwrap_or_else(|e| e.into_inner());
    guard.get(key).cloned().filter(|v| !v.trim().is_empty())
}

pub fn remove_runtime_secret(key: &str) {
    let mut guard = runtime_secrets().write().unwrap_or_else(|e| e.into_inner());
    guard.remove(key);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_secret_set_get_remove_round_trip() {
        let key = "test_google_secret_round_trip";
        remove_runtime_secret(key);

        set_runtime_secret(key, "value-123");
        assert_eq!(get_runtime_secret(key), Some("value-123".to_string()));

        remove_runtime_secret(key);
        assert_eq!(get_runtime_secret(key), None);
    }

    #[test]
    fn get_runtime_secret_returns_none_for_empty_values() {
        let key = "test_google_secret_empty";
        remove_runtime_secret(key);

        set_runtime_secret(key, "");
        assert_eq!(get_runtime_secret(key), None);

        set_runtime_secret(key, "   ");
        assert_eq!(get_runtime_secret(key), None);

        remove_runtime_secret(key);
    }
}
