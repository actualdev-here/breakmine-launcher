//! GPUI profiles and credentials are stored separately from Tauri.
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
    if name.is_empty() || token.is_empty() {
        return Err("Invalid account information.".into());
    }
    let mut accounts = load(path)?;
    let id = match accounts
        .online_profiles
        .iter_mut()
        .find(|profile| profile.name.eq_ignore_ascii_case(name) && profile.api == api)
    {
        Some(profile) => {
            profile.name = name.into();
            profile.token = token;
            profile.expires = expires;
            profile.api = api.into();
            profile.id.clone()
        }
        None => {
            let id = crate::util::new_id("gpui-oauth");
            accounts.online_profiles.push(OnlineProfile {
                id: id.clone(),
                name: name.into(),
                token,
                expires,
                api: api.into(),
            });
            id
        }
    };
    accounts.active_account = id;
    save(path, &accounts)?;
    Ok(accounts)
}

pub fn load(path: &Path) -> Result<Accounts, String> {
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Couldn't read GPUI accounts: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Accounts::default()),
        Err(e) => Err(format!("Couldn't read GPUI accounts: {e}")),
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
    let mut accounts = load(path)?;
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
    let mut accounts = load(path)?;
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
    let mut accounts = load(path)?;
    let index = accounts
        .online_profiles
        .iter()
        .position(|profile| profile.id == id)
        .ok_or("Online profile not found in GPUI.")?;
    let removed = accounts.online_profiles.remove(index);
    if accounts.active_account == id {
        accounts.active_account.clear();
    }
    save(path, &accounts)?;
    Ok((accounts, removed))
}

pub fn remove_offline(path: &Path, id: &str) -> Result<Accounts, String> {
    let _guard = ACCOUNT_WRITE_LOCK
        .lock()
        .map_err(|_| "Account storage is unavailable.")?;
    let mut accounts = load(path)?;
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
    #[test]
    fn oauth_profile_is_private_and_reconnect_updates_existing_account() {
        let dir = std::env::temp_dir().join(crate::util::new_id("oauth-account-test"));
        let path = dir.join("accounts.json");
        let first = upsert_oauth(
            &path,
            "Alex",
            "old-token".into(),
            100,
            "https://api.example.com",
        )
        .unwrap();
        let second = upsert_oauth(
            &path,
            "alex",
            "new-token".into(),
            200,
            "https://api.example.com",
        )
        .unwrap();
        assert_eq!(first.active_account, second.active_account);
        assert_eq!(second.online_profiles.len(), 1);
        assert_eq!(load(&path).unwrap().online_profiles[0].token, "new-token");
        let (remaining, removed) = remove_online(&path, &second.active_account).unwrap();
        assert!(remaining.online_profiles.is_empty());
        assert!(remaining.active_account.is_empty());
        assert_eq!(removed.api, "https://api.example.com");
        assert!(!fs::read_to_string(&path).unwrap().contains("new-token"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn offline_accounts_validate_names_and_remain_isolated() {
        let dir = std::env::temp_dir().join(crate::util::new_id("accounts-test"));
        let path = dir.join("accounts.json");
        assert!(add_offline(&path, "../bad").is_err());
        assert!(add_offline(&path, "no").is_err());
        let a = add_offline(&path, "Alex").unwrap();
        assert_eq!(a.offline_profiles.len(), 1);
        let again = add_offline(&path, "alex").unwrap();
        assert_eq!(again.offline_profiles.len(), 1);
        assert_eq!(again.active_account, a.active_account);
        let b = select(&path, "tauri-account-123").unwrap();
        assert_eq!(b.active_account, "tauri-account-123");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(!fs::read_to_string(&path).unwrap().contains("token"));
        assert!(remove_offline(&path, "tauri-account-123").is_err());
        let c = remove_offline(&path, &a.active_account).unwrap();
        assert_eq!(c.active_account, "tauri-account-123");
        fs::write(&path, "not json").unwrap();
        assert!(add_offline(&path, "Steve").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
        fs::remove_dir_all(dir).unwrap();
    }
}
