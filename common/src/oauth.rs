//! The browser login, with a Control Panel module's app and proxy: the
//! default browser (where the person is usually logged in already) opens the
//! service's authorize page; they click Authorize; the service sends a code
//! to http://localhost:52246/, caught here; the module's Google Apps Script
//! proxy, which holds the app's secret, trades it for tokens. Refreshing goes
//! through the proxy too.
//!
//! The tokens are kept in the plugin's settings folder (token.json).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Where the modules' apps send the code (registered with Twitch and Patreon)
pub const REDIRECT_URI: &str = "http://localhost:52246/";
const REDIRECT_PORT: u16 = 52246;
/// How long the browser has to come back
const WAIT: Duration = Duration::from_secs(300);
const TOKEN_FILE: &str = "token.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds (0: unknown)
    pub expires_at: u64,
    pub scope: Vec<String>,
}

impl Token {
    pub fn is_empty(&self) -> bool {
        self.access_token.is_empty()
    }

    /// Expired, or expiring in the next minute
    pub fn stale(&self) -> bool {
        self.expires_at != 0 && crate::unix_secs() + 60 >= self.expires_at
    }

    /// From the service's token answer (access_token, refresh_token, expires_in, scope)
    fn from_answer(v: &Value, old_refresh: &str) -> Result<Token, String> {
        let access = v.get("access_token").and_then(Value::as_str).unwrap_or("");
        if access.is_empty() {
            let why = v
                .get("error_description")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("error"))
                .map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string()))
                .unwrap_or_else(|| v.to_string());
            return Err(why);
        }
        let scope = match v.get("scope") {
            Some(Value::Array(a)) => a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect(),
            Some(Value::String(s)) => s.split_whitespace().map(str::to_string).collect(),
            _ => Vec::new(),
        };
        let expires_in = v.get("expires_in").and_then(Value::as_u64).unwrap_or(0);
        Ok(Token {
            access_token: access.to_string(),
            refresh_token: v.get("refresh_token").and_then(Value::as_str).filter(|r| !r.is_empty()).unwrap_or(old_refresh).to_string(),
            expires_at: if expires_in > 0 { crate::unix_secs() + expires_in } else { 0 },
            scope,
        })
    }
}

pub fn load_token() -> Token {
    crate::load_json(TOKEN_FILE)
}

pub fn save_token(token: &Token) {
    crate::save_json(TOKEN_FILE, token);
}

pub fn forget_token() {
    let _ = std::fs::remove_file(crate::config_dir().join(TOKEN_FILE));
}

/// A login: the service's authorize page and the module's proxy
pub struct Login {
    /// e.g. "https://id.twitch.tv/oauth2/authorize"
    pub authorize: &'static str,
    pub client_id: &'static str,
    pub scopes: Vec<&'static str>,
    /// The module's proxy (crate::proxies)
    pub proxy: &'static str,
}

/// Something unguessable for the login's `state`
fn state() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mixed = nanos ^ ((std::process::id() as u128) << 64) ^ (&nanos as *const u128 as usize as u128);
    format!("{:032x}", mixed.wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835))
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 2;
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A query's value ("/?code=x&state=y", "code" → "x")
pub fn query_value(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then(|| decode(v))
    })
}

pub fn authorize_url(login: &Login, state: &str) -> String {
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
        login.authorize,
        encode(login.client_id),
        encode(REDIRECT_URI),
        encode(&login.scopes.join(" ")),
        state
    )
}

const PAGE: &str = "<html><head><meta charset='utf-8'><title>Theater</title><style>\
body{font-family:'Segoe UI',Roboto,Arial,sans-serif;background:#121212;color:#fff;text-align:center;padding-top:100px}\
.box{max-width:520px;margin:0 auto;padding:40px;background:#1e1e1e;border-radius:8px}h1{color:#4caf50}p{color:#b0b0b0;font-size:1.1em}\
</style></head><body><div class='box'><h1>%TITLE%</h1><p>%TEXT%</p></div></body></html>";

fn page(title: &str, text: &str) -> String {
    let body = PAGE.replace("%TITLE%", title).replace("%TEXT%", text);
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
}

/// Logs in: opens the browser, waits for the code, trades it. The tokens
/// are saved.
pub async fn login(login: &Login) -> Result<Token, String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .await
        .map_err(|e| format!("port {REDIRECT_PORT} is busy (is another login open, in the Control Panel or another Theater plugin?): {e}"))?;
    let state = state();
    crate::obs::open_url(&authorize_url(login, &state));
    let code = tokio::time::timeout(WAIT, async {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { continue };
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = request.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("").to_string();
            // Browsers also ask for /favicon.ico
            if query_value(&path, "state").is_none() && query_value(&path, "error").is_none() {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            }
            let answer = if let Some(error) = query_value(&path, "error") {
                let why = query_value(&path, "error_description").unwrap_or(error);
                (Err(format!("the login was refused: {why}")), page("Not logged in", &format!("{why}. You can close this tab.")))
            } else if query_value(&path, "state").as_deref() != Some(state.as_str()) {
                (Err("the login's answer didn't match (try again)".to_string()), page("Not logged in", "This answer wasn't for this login. Try again from OBS."))
            } else {
                match query_value(&path, "code").filter(|c| !c.is_empty()) {
                    Some(code) => (Ok(code), page("Logged in!", "Theater has your authorization. You can close this tab and go back to OBS.")),
                    None => (Err("no code came back".to_string()), page("Not logged in", "No authorization came back. Try again from OBS.")),
                }
            };
            let _ = stream.write_all(answer.1.as_bytes()).await;
            let _ = stream.flush().await;
            return answer.0;
        }
    })
    .await
    .map_err(|_| "nobody authorized it in the browser within 5 minutes".to_string())??;
    drop(listener);
    let token = proxy(login.proxy, json!({ "action": "exchange", "code": code, "redirect_uri": REDIRECT_URI }), "").await?;
    save_token(&token);
    Ok(token)
}

/// A new access token from the refresh token (saved)
pub async fn refresh(proxy_url: &str, token: &Token) -> Result<Token, String> {
    if token.refresh_token.is_empty() {
        return Err("no refresh token: log in again".into());
    }
    let new = proxy(proxy_url, json!({ "action": "refresh", "refresh_token": token.refresh_token }), &token.refresh_token).await?;
    save_token(&new);
    Ok(new)
}

/// A request to a module's proxy (text/plain JSON, as the modules send it;
/// Apps Script answers through a redirect, which is followed)
async fn proxy(url: &str, body: Value, old_refresh: &str) -> Result<Token, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .header("Content-Type", "text/plain")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("couldn't reach the login proxy: {e}"))?;
    let text = resp.text().await.map_err(|e| format!("the login proxy's answer broke off: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|_| format!("the login proxy answered something else: {}", text.chars().take(160).collect::<String>()))?;
    Token::from_answer(&v, old_refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_redirect_and_builds_the_authorize_page() {
        assert_eq!(query_value("/?code=ab%2Fc&state=s1&scope=a+b", "code").as_deref(), Some("ab/c"));
        assert_eq!(query_value("/?code=x&scope=a+b", "scope").as_deref(), Some("a b"));
        assert_eq!(query_value("/favicon.ico", "code"), None);
        let login = Login { authorize: "https://id.twitch.tv/oauth2/authorize", client_id: "id1", scopes: vec!["bits:read", "channel:read:redemptions"], proxy: "" };
        let url = authorize_url(&login, "st");
        assert_eq!(
            url,
            "https://id.twitch.tv/oauth2/authorize?response_type=code&client_id=id1&redirect_uri=http%3A%2F%2Flocalhost%3A52246%2F&scope=bits%3Aread%20channel%3Aread%3Aredemptions&state=st"
        );
        assert_ne!(state(), state(), "a new state each login");
    }

    #[test]
    fn reads_token_answers_and_their_errors() {
        let t = Token::from_answer(&json!({"access_token":"a","refresh_token":"r","expires_in":3600,"scope":["x","y"]}), "").unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_str(), t.scope.len()), ("a", "r", 2));
        assert!(!t.stale());
        // Patreon: scope as text; a refresh that doesn't send a new refresh token keeps the old one
        let p = Token::from_answer(&json!({"access_token":"b","expires_in":1,"scope":"identity campaigns"}), "old").unwrap();
        assert_eq!((p.refresh_token.as_str(), p.scope.len()), ("old", 2));
        assert!(p.stale());
        assert_eq!(Token::from_answer(&json!({"error":"invalid_grant","error_description":"Invalid refresh token"}), "").unwrap_err(), "Invalid refresh token");
    }
}
