//! The breakmine.com API for password sign-in, skin uploads, and capes.
use serde_json::Value;
use std::time::Duration;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(8)).timeout_read(Duration::from_secs(30)).user_agent("Breakmine-Launcher").build()
}

pub struct Login {
    pub token: String,
    pub expires: u64,
    pub created: bool,
}

/// "Login or Register": sign in, creating the account first if the name is free.
pub fn login_or_register(api: &str, username: &str, password: &str) -> Result<Login, String> {
    let a = agent();
    let mut created = false;
    match a.get(&endpoint(api, &format!("/api/user/{username}"))).call() {
        Ok(_) => {}
        Err(ureq::Error::Status(404, _)) => {
            a.post(&endpoint(api, "/api/register"))
                .send_json(serde_json::json!({ "username": username, "password": password }))
                .map_err(api_error)?;
            created = true;
        }
        Err(e) => return Err(api_error(e)),
    }
    let v: Value = a
        .post(&endpoint(api, "/api/login"))
        .send_json(serde_json::json!({ "username": username, "password": password }))
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => "Wrong password for that account.".into(),
            e => api_error(e),
        })?
        .into_json()
        .map_err(|e| e.to_string())?;
    let token = v.get("token").and_then(|t| t.as_str()).ok_or("breakmine.com didn't return a token.")?.to_string();
    let expires = crate::util::now() + parse_lifetime(v.get("expiresIn").and_then(|x| x.as_str()).unwrap_or("7d"));
    Ok(Login { token, expires, created })
}

/// "7d", "12h", "30m", "3600" → seconds.
fn parse_lifetime(s: &str) -> u64 {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = n.parse().unwrap_or(7);
    match unit {
        "d" => n * 86400,
        "h" => n * 3600,
        "m" => n * 60,
        "" => n,
        _ => 7 * 86400,
    }
}

fn api_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(429, _) => "Too many attempts. Wait a minute and try again.".into(),
        ureq::Error::Status(code, r) => r
            .into_json::<Value>()
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
            .unwrap_or_else(|| format!("breakmine.com answered with error {code}.")),
        ureq::Error::Transport(_) => "Can't reach breakmine.com. Check your connection.".into(),
    }
}

fn endpoint(api: &str, path: &str) -> String {
    format!("{}{}", api.trim_end_matches('/'), path)
}

pub fn upload_skin(api: &str, token: &str, png: &[u8]) -> Result<(), String> {
    if png.len() > 256 * 1024 {
        return Err("Skins must be under 256 KB.".into());
    }
    let boundary = format!("----breakmine{}", crate::util::new_id("b").replace('-', ""));
    let mut body = Vec::with_capacity(png.len() + 256);
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes());
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    agent()
        .post(&endpoint(api, "/api/upload_skin"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .send_bytes(&body)
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => "Your breakmine.com session expired. Sign in again.".into(),
            e => api_error(e),
        })?;
    Ok(())
}

fn bearer_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401, _) => "Your breakmine.com session expired. Sign in again.".into(),
        e => api_error(e),
    }
}

pub fn capes(api: &str, token: &str) -> Result<Value, String> {
    agent()
        .get(&endpoint(api, "/api/capes"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(bearer_err)?
        .into_json()
        .map_err(|e| e.to_string())
}

pub fn choose_cape(api: &str, token: &str, cape_id: Option<&str>) -> Result<(), String> {
    agent()
        .post(&endpoint(api, "/api/cape/choice"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "capeId": cape_id }))
        .map_err(bearer_err)?;
    Ok(())
}

pub fn redeem_cape(api: &str, token: &str, code: &str) -> Result<Value, String> {
    agent()
        .post(&endpoint(api, "/api/capes/redeem"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "code": code }))
        .map_err(bearer_err)?
        .into_json()
        .map_err(|e| e.to_string())
}
