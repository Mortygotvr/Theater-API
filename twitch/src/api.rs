//! Twitch's API with the twitch_api module's app: the login (oauth,
//! through the module's proxy), the account it's for, and Helix: rewards,
//! subscribers, followers, chat messages and EventSub subscriptions. An
//! expired token is refreshed (through the proxy) and the call made again.

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use theater_api_common::oauth::{self, Login, Token};

/// The twitch_api module's app (twitch_api.tcp)
pub const CLIENT_ID: &str = "8di1qtifccgzrs1jvb6t6ddo8z9crv";
const HELIX: &str = "https://api.twitch.tv/helix";

/// What it asks Twitch for, and why
pub const SCOPES: [&str; 8] = [
    "channel:read:redemptions",   // redeems, the reward list
    "channel:manage:redemptions", // rewards made from Theater: pause, refund
    "bits:read",                  // cheers
    "channel:read:subscriptions", // subs, resubs, gifts, the subscriber list
    "moderator:read:followers",   // follows, the follower list
    "channel:read:hype_train",    // hype trains
    "channel:moderate",           // bans and timeouts
    "user:write:chat",            // sending a chat message
];

pub fn login_spec() -> Login {
    Login { authorize: "https://id.twitch.tv/oauth2/authorize", client_id: CLIENT_ID, scopes: SCOPES.to_vec(), proxy: theater_api_common::proxies::TWITCH }
}

/// The logged-in account
#[derive(Clone, Debug, Default, Serialize)]
pub struct Account {
    pub user_id: String,
    pub login: String,
    pub name: String,
}

static TOKEN: Mutex<Option<Token>> = Mutex::new(None);
static ACCOUNT: Mutex<Option<Account>> = Mutex::new(None);

pub fn account() -> Option<Account> {
    ACCOUNT.lock().clone()
}

pub fn logged_in() -> bool {
    TOKEN.lock().as_ref().is_some_and(|t| !t.is_empty())
}

/// The scopes the token has that the plugin asks for, and those it lacks
pub fn missing_scopes() -> Vec<&'static str> {
    let token = TOKEN.lock().clone().unwrap_or_default();
    SCOPES.iter().copied().filter(|s| !token.scope.iter().any(|t| t == s)).collect()
}

/// The saved login (at load)
pub fn load() {
    let token = oauth::load_token();
    if !token.is_empty() {
        *TOKEN.lock() = Some(token);
    }
}

pub fn set_token(token: Token) {
    *TOKEN.lock() = Some(token);
}

pub fn forget() {
    *TOKEN.lock() = None;
    *ACCOUNT.lock() = None;
    oauth::forget_token();
}

pub fn access_token() -> Option<String> {
    TOKEN.lock().as_ref().filter(|t| !t.is_empty()).map(|t| t.access_token.clone())
}

fn client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().unwrap_or_default()).clone()
}

/// A new access token (after a 401, or when it's about to expire)
async fn refresh() -> Result<(), String> {
    let token = TOKEN.lock().clone().ok_or("not logged in")?;
    match oauth::refresh(theater_api_common::proxies::TWITCH, &token).await {
        Ok(new) => {
            *TOKEN.lock() = Some(new);
            theater_api_common::log("🔑 Refreshed the Twitch login");
            Ok(())
        }
        Err(e) => {
            theater_api_common::log(&format!("⚠️ Couldn't refresh the Twitch login: {e}"));
            Err(format!("the login expired and couldn't be renewed ({e}): log in again"))
        }
    }
}

/// Who the token is for (id.twitch.tv's validate); refreshes first when stale
pub async fn validate() -> Result<Account, String> {
    if TOKEN.lock().as_ref().is_some_and(|t| t.stale()) {
        refresh().await?;
    }
    for attempt in 0..2 {
        let token = access_token().ok_or("not logged in")?;
        let resp = client()
            .get("https://id.twitch.tv/oauth2/validate")
            .header("Authorization", format!("OAuth {token}"))
            .send()
            .await
            .map_err(|e| format!("couldn't reach Twitch: {e}"))?;
        if resp.status().as_u16() == 401 && attempt == 0 {
            refresh().await?;
            continue;
        }
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        let user_id = v.get("user_id").and_then(Value::as_str).unwrap_or("").to_string();
        if user_id.is_empty() {
            return Err(format!("Twitch didn't accept the login: {}", v.get("message").and_then(Value::as_str).unwrap_or("log in again")));
        }
        // The scopes as Twitch has them now
        if let Some(scopes) = v.get("scopes").and_then(Value::as_array) {
            if let Some(t) = TOKEN.lock().as_mut() {
                t.scope = scopes.iter().filter_map(|s| s.as_str().map(str::to_string)).collect();
            }
        }
        let login = v.get("login").and_then(Value::as_str).unwrap_or("").to_string();
        let mut account = Account { user_id, name: login.clone(), login };
        if let Ok(users) = helix("GET", &format!("/users?id={}", account.user_id), None).await {
            if let Some(name) = users.pointer("/data/0/display_name").and_then(Value::as_str) {
                account.name = name.to_string();
            }
        }
        *ACCOUNT.lock() = Some(account.clone());
        return Ok(account);
    }
    Err("Twitch didn't accept the login: log in again".into())
}

/// A Helix call (`path` from /helix on), refreshing the token once on 401.
/// Errors carry Twitch's message.
pub async fn helix(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    for attempt in 0..2 {
        let token = access_token().ok_or("not logged in")?;
        let url = format!("{HELIX}{path}");
        let mut req = match method {
            "POST" => client().post(&url),
            "PATCH" => client().patch(&url),
            "DELETE" => client().delete(&url),
            _ => client().get(&url),
        }
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("Bearer {token}"));
        if let Some(body) = &body {
            req = req.json(body);
        }
        let resp = req.send().await.map_err(|e| format!("couldn't reach Twitch: {e}"))?;
        let status = resp.status().as_u16();
        if status == 401 && attempt == 0 {
            refresh().await?;
            continue;
        }
        let text = resp.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(v);
        }
        let why = v.get("message").and_then(Value::as_str).unwrap_or(&text);
        return Err(format!("{status}: {why}"));
    }
    Err("Twitch refused the login: log in again".into())
}

/// Every page of a list call (`path` with its own query), up to `max` items
pub async fn helix_all(path: &str, max: usize) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut cursor = String::new();
    loop {
        let sep = if path.contains('?') { '&' } else { '?' };
        let page = if cursor.is_empty() { format!("{path}{sep}first=100") } else { format!("{path}{sep}first=100&after={cursor}") };
        let v = helix("GET", &page, None).await?;
        if let Some(items) = v.get("data").and_then(Value::as_array) {
            out.extend(items.iter().cloned());
        }
        cursor = v.pointer("/pagination/cursor").and_then(Value::as_str).unwrap_or("").to_string();
        if cursor.is_empty() || out.len() >= max {
            return Ok(out);
        }
    }
}

/// Subscribes this EventSub session to `kind`
pub async fn subscribe(session_id: &str, kind: &str, version: &str, condition: Value) -> Result<(), String> {
    let body = json!({ "type": kind, "version": version, "condition": condition, "transport": { "method": "websocket", "session_id": session_id } });
    helix("POST", "/eventsub/subscriptions", Some(body)).await.map(|_| ())
}

/// Sends a chat message as the logged-in account, in its own channel
pub async fn send_chat(message: &str) -> Result<(), String> {
    let account = account().ok_or("not logged in")?;
    let v = helix("POST", "/chat/messages", Some(json!({ "broadcaster_id": account.user_id, "sender_id": account.user_id, "message": message }))).await?;
    match v.pointer("/data/0/is_sent").and_then(Value::as_bool) {
        Some(false) => Err(v.pointer("/data/0/drop_reason/message").and_then(Value::as_str).unwrap_or("Twitch dropped it").to_string()),
        _ => Ok(()),
    }
}
