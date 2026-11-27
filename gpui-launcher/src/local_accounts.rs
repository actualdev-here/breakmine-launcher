//! GPUI profiles and credentials are stored separately from Tauri.
use crate::credential_store::{CredentialStore, SystemStore};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, sync::Mutex};

static ACCOUNT_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Accounts {
    pub active_account: String,
    pub offline_profiles: Vec<OfflineProfile>,
    pub online_profiles: Vec<OnlineProfile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineProfile {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineProfile {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing)]
    pub token: String,
    pub expires: u64,
    #[serde(default)]
    pub api: String,
}

pub fn upsert_oauth(
    path: &Path,
    name: &str,
    token: String,
    expires: u64,
    api: &str,
) -> Result<Accounts, String> {
    let _guard = ACCOUNT_WRITE_LOCK
        .lock()
        .map_err(|_| "Account storage is unavailable.")?;
    upsert_with(path, name, token, expires, api, &SystemStore)
}

fn upsert_with(
    path: &Path,
    name: &str,
    token: String,
    expires: u64,
    api: &str,
    store: &impl CredentialStore,
) -> Result<Accounts, String> {
    if name.is_empty() || token.is_empty() {
        return Err("Invalid account information.".into());
    }
    let mut accounts = load_with(path, store)?;
    let index = accounts
        .online_profiles
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(name) && p.api == api);
    let id = index
        .map(|i| accounts.online_profiles[i].id.clone())
        .unwrap_or_else(|| crate::util::new_id("gpui-oauth"));
    let previous = store.get(&id)?;
    store.set(&id, &token)?;
    let profile = OnlineProfile {
        id: id.clone(),
        name: name.into(),
        token,
        expires,
        api: api.into(),
    };
    if let Some(i) = index {
        accounts.online_profiles[i] = profile;
    } else {
        accounts.online_profiles.push(profile);
    }
    accounts.active_account = id.clone();
    if let Err(error) = save(path, &accounts) {
        let restored = match previous {
            Some(token) => store.set(&id, &token),
            None => store.delete(&id),
        };
        return Err(if restored.is_err() {
            format!("{error} Secure storage could not be restored; reconnect this account.")
        } else {
            error
        });
    }
    Ok(accounts)
}

pub fn load(path: &Path) -> Result<Accounts, String> {
    let _guard = ACCOUNT_WRITE_LOCK
        .lock()
        .map_err(|_| "Account storage is unavailable.")?;
    load_with(path, &SystemStore)
}

fn read_metadata(path: &Path) -> Result<Accounts, String> {
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Couldn't read GPUI accounts: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Accounts::default()),
        Err(e) => Err(format!("Couldn't read GPUI accounts: {e}")),
    }
}

/// Read account names without exposing tokens when the credential store is locked.
pub fn load_metadata(path: &Path) -> Result<Accounts, String> {
    let _guard = ACCOUNT_WRITE_LOCK
        .lock()
        .map_err(|_| "Account storage is unavailable.")?;
    let mut accounts = read_metadata(path)?;
    for account in &mut accounts.online_profiles {
        account.token.clear();
    }
    Ok(accounts)
}

fn load_with(path: &Path, store: &impl CredentialStore) -> Result<Accounts, String> {
    let mut accounts = read_metadata(path)?;
    let mut migrated = false;
    for profile in &mut accounts.online_profiles {
        if !profile.token.is_empty() {
            // Keep the original file intact until every legacy token is stored successfully.
            store.set(&profile.id, &profile.token)?;
            migrated = true;
        } else {
            profile.token = store.get(&profile.id)?.unwrap_or_default();
        }
    }
    if migrated {
        save(path, &accounts)?;
    }
    Ok(accounts)
}
// Offline profiles can still be managed if a previously migrated keyring is locked.
// A legacy plaintext file must not be rewritten until its tokens have been migrated.
fn load_allow_offline(
    path: &Path,
    store: &impl CredentialStore,
) -> Result<(Accounts, Option<String>), String> {
    match load_with(path, store) {
        Ok(accounts) => Ok((accounts, None)),
        Err(error) => {
            let accounts = read_metadata(path)?;
            if accounts
                .online_profiles
                .iter()
                .any(|profile| !profile.token.is_empty())
            {
                return Err(error);
            }
            Ok((accounts, Some(error)))
        }
    }
}

fn save(path: &Path, accounts: &Accounts) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid account storage path")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(
        ".gpui-accounts-{}.tmp",
        crate::util::new_id("save")
    ));
    let result = (|| {
        use std::io::Write;
        let data = serde_json::to_vec_pretty(accounts).map_err(|e| e.to_string())?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
        file.write_all(&data).map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&tmp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub fn select(path: &Path, id: &str) -> Result<Accounts, String> {
    let _guard = ACCOUNT_WRITE_LOCK
        .lock()
        .map_err(|_| "Account storage is unavailable.")?;
    if id.is_empty() || id.len() > 128 {
        return Err("Invalid account ID.".into());
    }
    let (mut accounts, keyring_error) = load_allow_offline(path, &SystemStore)?;
    if accounts
        .online_profiles
        .iter()
        .any(|profile| profile.id == id)
    {
        if let Some(error) = keyring_error {
            return Err(error);
        }
    }
    accounts.active_account = id.into();
    save(path, &accounts)?;
    Ok(accounts)
}
pub fn add_offline(path: &Path, name: &str) -> Result<Accounts, String> {
    let _guard = ACCOUNT_WRITE_LOCK
        .lock()
        .map_err(|_| "Account storage is unavailable.")?;
    let name = name.trim();
    if !(3..=32).contains(&name.len())
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Usernames are 3–32 letters, numbers, _ or -.".into());
    }
    let (mut accounts, _) = load_allow_offline(path, &SystemStore)?;
    let id = match accounts
        .offline_profiles
        .iter()
        .find(|profile| profile.name.eq_ignore_ascii_case(name))
    {
        Some(profile) => profile.id.clone(),
        None => {
            let id = crate::util::new_id("gpui-offline");
            accounts.offline_profiles.push(OfflineProfile {
                id: id.clone(),
                name: name.into(),
            });
            id
        }
    };
    accounts.active_account = id;
    save(path, &accounts)?;
    Ok(accounts)
}
pub fn remove_online(path: &Path, id: &str) -> Result<(Accounts, OnlineProfile), String> {
    let _guard = ACCOUNT_WRITE_LOCK
        .lock()
        .map_err(|_| "Account storage is unavailable.")?;
    remove_online_with(path, id, &SystemStore)
}

fn remove_online_with(
    path: &Path,
    id: &str,
    store: &impl CredentialStore,
) -> Result<(Accounts, OnlineProfile), String> {
    let mut accounts = load_with(path, store)?;
    let index = accounts
        .online_profiles
        .iter()
        .position(|profile| profile.id == id)
        .ok_or("Online profile not found in GPUI.")?;
    let removed = accounts.online_profiles.remove(index);
    if accounts.active_account == id {
        accounts.active_account.clear();
    }
    store.delete(id)?;
    if let Err(error) = save(path, &accounts) {
        if !removed.token.is_empty() && store.set(id, &removed.token).is_err() {
            return Err(format!(
                "{error} Secure storage could not be restored; reconnect this account."
            ));
        }
        return Err(error);
    }
    Ok((accounts, removed))
}

pub fn remove_offline(path: &Path, id: &str) -> Result<Accounts, String> {
    let _guard = ACCOUNT_WRITE_LOCK
        .lock()
        .map_err(|_| "Account storage is unavailable.")?;
    let (mut accounts, _) = load_allow_offline(path, &SystemStore)?;
    let before = accounts.offline_profiles.len();
    accounts.offline_profiles.retain(|profile| profile.id != id);
    if before == accounts.offline_profiles.len() {
        return Err("Offline profile not found in GPUI.".into());
    }
    if accounts.active_account == id {
        accounts.active_account.clear();
    }
    save(path, &accounts)?;
    Ok(accounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    #[derive(Default)]
    struct FakeStore {
        tokens: Mutex<HashMap<String, String>>,
        fail: bool,
    }
    impl CredentialStore for FakeStore {
        fn get(&self, id: &str) -> Result<Option<String>, String> {
            if self.fail {
                return Err("Keyring unavailable".into());
            }
            Ok(self.tokens.lock().unwrap().get(id).cloned())
        }
        fn set(&self, id: &str, token: &str) -> Result<(), String> {
            if self.fail {
                return Err("Keyring unavailable".into());
            }
            self.tokens.lock().unwrap().insert(id.into(), token.into());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            if self.fail {
                return Err("Keyring unavailable".into());
            }
            self.tokens.lock().unwrap().remove(id);
            Ok(())
        }
    }
    fn temp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(crate::util::new_id("secure-account-test"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_reconnects_and_removes_without_plaintext() {
        let dir = temp();
        let path = dir.join("accounts.json");
        let store = FakeStore::default();
        let first = upsert_with(
            &path,
            "Alex",
            "old-token".into(),
            100,
            "https://api.example.com",
            &store,
        )
        .unwrap();
        let second = upsert_with(
            &path,
            "alex",
            "new-token".into(),
            200,
            "https://api.example.com",
            &store,
        )
        .unwrap();
        assert_eq!(first.active_account, second.active_account);
        assert_eq!(
            load_with(&path, &store).unwrap().online_profiles[0].token,
            "new-token"
        );
        let disk = fs::read_to_string(&path).unwrap();
        assert!(
            !disk.contains("new-token")
                && !disk.contains("old-token")
                && !disk.contains("\"token\"")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let (remaining, removed) =
            remove_online_with(&path, &second.active_account, &store).unwrap();
        assert_eq!(removed.token, "new-token");
        assert!(remaining.online_profiles.is_empty());
        assert!(store.get(&second.active_account).unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migrates_legacy_tokens_only_after_secure_save() {
        let dir = temp();
        let path = dir.join("accounts.json");
        let legacy = r#"{"activeAccount":"a","onlineProfiles":[{"id":"a","name":"Alex","token":"legacy-secret","expires":123,"api":"https://api.example.com"}]}"#;
        fs::write(&path, legacy).unwrap();
        let unavailable = FakeStore {
            fail: true,
            ..Default::default()
        };
        assert!(load_with(&path, &unavailable).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
        let store = FakeStore::default();
        assert_eq!(
            load_with(&path, &store).unwrap().online_profiles[0].token,
            "legacy-secret"
        );
        assert_eq!(store.get("a").unwrap().as_deref(), Some("legacy-secret"));
        assert!(!fs::read_to_string(&path).unwrap().contains("legacy-secret"));
        assert_eq!(
            load_with(&path, &store).unwrap().online_profiles[0].token,
            "legacy-secret"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unavailable_store_never_writes_new_token() {
        let dir = temp();
        let path = dir.join("accounts.json");
        let store = FakeStore {
            fail: true,
            ..Default::default()
        };
        assert!(upsert_with(
            &path,
            "Alex",
            "secret".into(),
            100,
            "https://api.example.com",
            &store
        )
        .is_err());
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn locked_keyring_keeps_migrated_offline_metadata_accessible() {
        let dir = temp();
        let path = dir.join("accounts.json");
        let good = FakeStore::default();
        upsert_with(
            &path,
            "Alex",
            "secret".into(),
            100,
            "https://api.example.com",
            &good,
        )
        .unwrap();
        let locked = FakeStore {
            fail: true,
            ..Default::default()
        };
        let (metadata, warning) = load_allow_offline(&path, &locked).unwrap();
        assert_eq!(metadata.online_profiles.len(), 1);
        assert!(metadata.online_profiles[0].token.is_empty());
        assert!(warning.is_some());
        let legacy = r#"{"onlineProfiles":[{"id":"a","name":"Alex","token":"legacy-secret","expires":123}]}"#;
        fs::write(&path, legacy).unwrap();
        assert!(load_allow_offline(&path, &locked).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn offline_accounts_work_without_a_keyring() {
        let dir = temp();
        let path = dir.join("accounts.json");
        assert!(add_offline(&path, "../bad").is_err());
        let a = add_offline(&path, "Alex").unwrap();
        let again = add_offline(&path, "alex").unwrap();
        assert_eq!(again.active_account, a.active_account);
        assert_eq!(select(&path, "other").unwrap().active_account, "other");
        assert_eq!(
            remove_offline(&path, &a.active_account)
                .unwrap()
                .active_account,
            "other"
        );
        fs::write(&path, "not json").unwrap();
        assert!(add_offline(&path, "Steve").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
        fs::remove_dir_all(dir).unwrap();
    }
}
