//! Store account tokens in the operating system's credential manager.

pub trait CredentialStore {
    fn get(&self, id: &str) -> Result<Option<String>, String>;
    fn set(&self, id: &str, token: &str) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

pub struct SystemStore;

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
))]
impl SystemStore {
    fn entry(id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new("com.breakmine.launcher", id)
            .map_err(|_| "Couldn't open secure account storage.".to_string())
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
))]
impl CredentialStore for SystemStore {
    fn get(&self, id: &str) -> Result<Option<String>, String> {
        match Self::entry(id)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(
                "Couldn't access secure account storage. Unlock your system keyring and try again."
                    .into(),
            ),
        }
    }
    fn set(&self, id: &str, token: &str) -> Result<(), String> {
        Self::entry(id)?.set_password(token).map_err(|_| {
            "Couldn't save the account securely. Unlock your system keyring and try again.".into()
        })
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Couldn't remove the account from secure storage. Unlock your system keyring and try again.".into()),
        }
    }
}

// Never fall back to an in-memory mock or plaintext on unsupported platforms.
#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
)))]
impl CredentialStore for SystemStore {
    fn get(&self, _: &str) -> Result<Option<String>, String> {
        Err("Secure account storage is not supported on this system.".into())
    }
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("Secure account storage is not supported on this system.".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("Secure account storage is not supported on this system.".into())
    }
}

/// Move copied legacy account tokens into secure storage and hydrate them for use.
pub fn protect_legacy_accounts(value: &mut serde_json::Value) -> Result<bool, String> {
    protect_legacy_with(value, &SystemStore)
}

fn protect_legacy_with(
    value: &mut serde_json::Value,
    store: &impl CredentialStore,
) -> Result<bool, String> {
    let mut changed = false;
    let Some(accounts) = value
        .get_mut("accounts")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(false);
    };
    for account in accounts {
        let Some(object) = account.as_object_mut() else {
            continue;
        };
        let token = object
            .get("token")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let offline = object
            .get("offline")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if offline && token.is_empty() {
            continue;
        }
        let id = object
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or("A legacy account has an invalid ID; its token was not moved.")?;
        let key = format!("legacy:{id}");
        if !token.is_empty() {
            if store.get(&key)?.as_deref() != Some(token.as_str()) {
                store.set(&key, &token)?;
            }
            changed = true;
        } else if let Some(token) = store.get(&key)? {
            object.insert("token".into(), token.into());
        }
    }
    Ok(changed)
}

/// Remove legacy tokens from the copy written to the GPUI settings file.
pub fn without_legacy_tokens(value: &serde_json::Value) -> serde_json::Value {
    let mut sanitized = value.clone();
    if let Some(accounts) = sanitized
        .get_mut("accounts")
        .and_then(serde_json::Value::as_array_mut)
    {
        for account in accounts {
            if let Some(object) = account.as_object_mut() {
                object.remove("token");
            }
        }
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};
    #[derive(Default)]
    struct FakeStore(Mutex<HashMap<String, String>>);
    impl CredentialStore for FakeStore {
        fn get(&self, id: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }
        fn set(&self, id: &str, token: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(id.into(), token.into());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }
    #[test]
    fn legacy_tokens_can_be_removed_from_json_and_restored() {
        let vault = FakeStore::default();
        let mut data = serde_json::json!({"accounts": [{"id":"a", "name":"Alex", "token":"secret"}], "settings": {"theme":"dark"}});
        assert!(protect_legacy_with(&mut data, &vault).unwrap());
        let mut safe = without_legacy_tokens(&data);
        assert!(!safe.to_string().contains("secret"));
        assert_eq!(safe["settings"]["theme"], "dark");
        assert!(!protect_legacy_with(&mut safe, &vault).unwrap());
        assert_eq!(safe["accounts"][0]["token"], "secret");
    }
    #[test]
    fn legacy_account_with_token_needs_a_valid_id() {
        let mut data = serde_json::json!({"accounts": [{"token":"secret"}]});
        assert!(protect_legacy_with(&mut data, &FakeStore::default()).is_err());
        assert_eq!(data["accounts"][0]["token"], "secret");
    }
    #[test]
    fn system_keyring_roundtrip_when_requested() {
        if std::env::var_os("BM_TEST_SYSTEM_KEYRING").is_none() {
            return;
        }
        let id = crate::util::new_id("keyring-test");
        let vault = SystemStore;
        for key in [id.clone(), format!("legacy:{id}")] {
            vault.set(&key, "test-only-secret").unwrap();
            assert_eq!(
                vault.get(&key).unwrap().as_deref(),
                Some("test-only-secret")
            );
            vault.delete(&key).unwrap();
            assert!(vault.get(&key).unwrap().is_none());
        }
    }
}
