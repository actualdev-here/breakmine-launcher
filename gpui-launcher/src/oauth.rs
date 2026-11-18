//! Breakmine OAuth profile authorization through a loopback callback and PKCE.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use url::Url;

pub struct Attempt {
    api: String,
    listener: TcpListener,
    redirect: String,
    client_id: String,
    verifier: String,
    state: String,
}
pub struct Credentials {
    pub name: String,
    pub token: String,
    pub expires: u64,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(15))
        .redirects(0)
        .user_agent("BreakmineLauncher-GPUI")
        .build()
}
fn api_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(429, _) => "Too many attempts. Wait a minute and try again.".into(),
        ureq::Error::Status(code, _) => format!("Breakmine answered with error {code}."),
        ureq::Error::Transport(_) => "Can't reach Breakmine. Check your connection.".into(),
    }
}
fn endpoint(api: &str, path: &str) -> String {
    format!("{}{}", api.trim_end_matches('/'), path)
}

pub fn revoke(api: &str, token: &str) -> Result<(), String> {
    let parsed = Url::parse(api).map_err(|_| "Invalid account API URL.")?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Invalid account API URL.".into());
    }
    agent()
        .post(&endpoint(api, "/oauth/revoke"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(api_error)?;
    Ok(())
}

pub fn prepare(api: &str) -> Result<(Attempt, String), String> {
    let parsed = Url::parse(api).map_err(|_| "Invalid account API URL.")?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("OAuth requires a valid HTTPS account API URL.".into());
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let redirect = format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr().map_err(|e| e.to_string())?.port()
    );
    let mut random = [0u8; 64];
    getrandom::getrandom(&mut random).map_err(|_| "Couldn't generate secure sign-in values.")?;
    let verifier = URL_SAFE_NO_PAD.encode(&random[..32]);
    let state = URL_SAFE_NO_PAD.encode(&random[32..]);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let reply: Value = agent().post(&endpoint(api, "/oauth/register")).send_json(json!({"client_name":"Breakmine Launcher", "redirect_uris":[redirect], "token_endpoint_auth_method":"none"})).map_err(api_error)?.into_json().map_err(|_| "Invalid OAuth registration response.")?;
    let client_id = reply["client_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or("OAuth registration returned no valid client ID.")?
        .to_string();
    let mut url =
        Url::parse(&endpoint(api, "/oauth/authorize")).map_err(|_| "Invalid authorization URL.")?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", "profile")
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Ok((
        Attempt {
            api: api.to_string(),
            listener,
            redirect,
            client_id,
            verifier,
            state,
        },
        url.to_string(),
    ))
}

fn callback_code(target: &str, expected: &str) -> Result<Option<String>, String> {
    let url = Url::parse(&format!("http://127.0.0.1{target}")).map_err(|_| "Invalid callback")?;
    if url.path() != "/callback" {
        return Err("Invalid callback".into());
    }
    let params: Vec<_> = url.query_pairs().collect();
    let values = |key: &str| {
        params
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_ref())
            .collect::<Vec<_>>()
    };
    let state = values("state");
    if state.len() != 1 || state[0] != expected {
        return Err("Invalid state".into());
    }
    let code = values("code");
    let errors = values("error");
    if !errors.is_empty() && code.is_empty() {
        return Ok(None);
    }
    if code.len() != 1 || code[0].is_empty() || !errors.is_empty() {
        return Err("Invalid callback".into());
    }
    Ok(Some(code[0].to_string()))
}
fn callback_page(stream: &mut TcpStream, success: bool) {
    let (title, message) = if success {
        ("Connected", "You can return to the launcher.")
    } else {
        (
            "Sign-in not completed",
            "You can return to the launcher and try again.",
        )
    };
    let body = format!("<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>body{{background:#090909;color:#eee;font:16px system-ui;display:grid;place-items:center;height:90vh}}main{{text-align:center}}p{{color:#aaa}}</style></head><body><main><h1>{title}</h1><p>{message}</p></main></body></html>");
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
}

pub fn finish(
    attempt: Attempt,
    cancel: Arc<AtomicBool>,
    save: impl FnOnce(Credentials) -> Result<(), String>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    let (code, mut stream) = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled.".into());
        }
        if Instant::now() >= deadline {
            return Err("Sign-in timed out. Please try again.".into());
        }
        let (mut stream, peer) = match attempt.listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut buf = [0u8; 8192];
        let Ok(n) = stream.read(&mut buf) else {
            continue;
        };
        let request = String::from_utf8_lossy(&buf[..n]);
        let Some(target) = request
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("GET "))
            .and_then(|line| line.split_whitespace().next())
        else {
            continue;
        };
        match callback_code(target, &attempt.state) {
            Ok(Some(code)) => break (code, stream),
            Ok(None) => {
                callback_page(&mut stream, false);
                return Err("Breakmine sign-in was cancelled or denied.".into());
            }
            Err(_) => continue,
        }
    };
    let mut issued_token: Option<String> = None;
    let result = (|| {
        if cancel.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled.".into());
        }
        let reply: Value = agent()
            .post(&endpoint(&attempt.api, "/oauth/token"))
            .send_form(&[
                ("grant_type", "authorization_code"),
                ("client_id", &attempt.client_id),
                ("code", &code),
                ("redirect_uri", &attempt.redirect),
                ("code_verifier", &attempt.verifier),
            ])
            .map_err(api_error)?
            .into_json()
            .map_err(|_| "Invalid OAuth token response.")?;
        let token = reply["access_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("OAuth returned no access token.")?
            .to_string();
        issued_token = Some(token.clone());
        if reply["scope"].as_str() != Some("profile") {
            return Err("Unexpected OAuth permissions returned by Breakmine.".into());
        }
        let info: Value = agent()
            .get(&endpoint(&attempt.api, "/oauth/userinfo"))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(api_error)?
            .into_json()
            .map_err(|_| "Invalid OAuth profile response.")?;
        if info["scope"].as_str() != Some("profile") {
            return Err("Unexpected OAuth permissions returned by Breakmine.".into());
        }
        let name = info["username"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 32)
            .ok_or("OAuth returned no valid username.")?
            .to_string();
        let expires = crate::util::now().saturating_add(reply["expires_in"].as_u64().unwrap_or(0));
        if cancel.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled.".into());
        }
        save(Credentials {
            name,
            token,
            expires,
        })
    })();
    if result.is_err() {
        if let Some(token) = issued_token {
            let _ = revoke(&attempt.api, &token);
        }
    }
    callback_page(&mut stream, result.is_ok());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_callback_state_and_rejects_ambiguous_parameters() {
        assert_eq!(
            callback_code("/callback?state=abc&code=xyz", "abc").unwrap(),
            Some("xyz".into())
        );
        assert_eq!(
            callback_code("/callback?state=abc&error=access_denied", "abc").unwrap(),
            None
        );
        assert!(callback_code("/callback?state=other&code=xyz", "abc").is_err());
        assert!(callback_code("/callback?state=abc&state=other&code=xyz", "abc").is_err());
        assert!(callback_code("/callback?state=abc&code=x&code=y", "abc").is_err());
        assert!(callback_code("/callback?state=abc&code=x&error=no", "abc").is_err());
        assert!(callback_code("/other?state=abc&code=x", "abc").is_err());
    }
    #[test]
    fn loopback_flow_exchanges_code_then_saves_profile() {
        use std::sync::mpsc;
        use std::thread;
        let api_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api_addr = api_listener.local_addr().unwrap();
        let api_thread = thread::spawn(move || {
            for step in 0..2 {
                let (mut stream, _) = api_listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0u8; 1024];
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let body = if step == 0 {
                    assert!(request.starts_with("POST /oauth/token "));
                    assert!(request.contains("code_verifier=test-verifier"));
                    r#"{"access_token":"test-token","scope":"profile","expires_in":3600}"#
                } else {
                    assert!(request.starts_with("GET /oauth/userinfo "));
                    assert!(request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer test-token"));
                    r#"{"username":"Alex","scope":"profile"}"#
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let callback_addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let attempt = Attempt {
            api: format!("http://{api_addr}"),
            listener,
            redirect: format!("http://{callback_addr}/callback"),
            client_id: "client".into(),
            verifier: "test-verifier".into(),
            state: "expected-state".into(),
        };
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            finish(attempt, Arc::new(AtomicBool::new(false)), |credentials| {
                tx.send((credentials.name, credentials.token)).unwrap();
                Ok(())
            })
        });
        let mut browser = TcpStream::connect(callback_addr).unwrap();
        browser
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            browser,
            "GET /callback?state=expected-state&code=auth-code HTTP/1.1\r\nHost: localhost\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        browser.read_to_string(&mut response).unwrap();
        assert!(response.contains("Connected"));
        assert_eq!(rx.recv().unwrap(), ("Alex".into(), "test-token".into()));
        assert!(worker.join().unwrap().is_ok());
        api_thread.join().unwrap();
    }

    #[test]
    fn refuses_insecure_account_api_before_network_request() {
        assert!(prepare("http://example.com").is_err());
        assert!(prepare("https://user:pass@example.com").is_err());
    }
}
