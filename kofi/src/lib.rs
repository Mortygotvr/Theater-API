//! Theater API · Ko-fi (theater-api-kofi.dll): Ko-fi's donations,
//! subscriptions, shop orders and commissions, from the kofi module's proxy
//! (Ko-fi's webhook posts to it; this asks it for what came), sent out through
//! the relay with the shared alert. Triggers: "Kofi <page> donation"…
//!
//! On OBS's global handlers:
//! - `theater_api_kofi_state(out string json)`: `{"polling","page","last_error","events"}`; the Control Panel's
//!   Ko-fi module stops asking the proxy itself when this is there (the
//!   proxy's queue empties on each read: two readers would split the donations)
//! - `theater_api_kofi_recent(out string json)`: the last supporters, newest first, `[{"user","amount","currency","type","message"}]`

pub mod events;
mod ui;

use std::collections::VecDeque;
use std::ffi::c_void;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use theater_api_common::{Info, obs_plugin};
use theater_shared::obs_link::{self, Calldata};

pub static INFO: Info = Info {
    id: "theater-api-kofi",
    name: "Theater API · Ko-fi",
    name_c: c"Theater API · Ko-fi",
    description_c: c"Ko-fi donations, subscriptions and shop orders for Theater",
    menu_title: c"\u{2615} Ko-fi API\u{2026}",
    window_title: "☕ Ko-fi API — Theater",
    icon_png: include_bytes!("../../assets/kofi.png"),
};

obs_plugin!(INFO, load: load, unload: unload);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Ko-fi's verification token (ko-fi.com/manage/webhooks): the proxy
    /// keeps each token's events apart
    pub verification_token: String,
    /// The trigger's channel: "Kofi <page> donation"
    pub page: String,
    pub poll_seconds: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { verification_token: String::new(), page: "kofi".into(), poll_seconds: 5 }
    }
}

impl Settings {
    pub fn url(&self) -> String {
        theater_api_common::proxies::KOFI.to_string()
    }
    pub fn page(&self) -> String {
        let page: String = self.page.split_whitespace().collect::<Vec<_>>().join("");
        if page.is_empty() { "kofi".into() } else { page }
    }
}

#[derive(Default)]
struct State {
    polling: bool,
    last_error: String,
    events: u64,
    seen: VecDeque<String>,
    recent: VecDeque<Value>,
}

static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);
static STATE: Mutex<Option<State>> = Mutex::new(None);
static TASK: Mutex<Option<tokio::task::JoinHandle<()>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut s = STATE.lock();
    f(s.get_or_insert_with(State::default))
}

pub fn settings() -> Settings {
    SETTINGS.lock().get_or_insert_with(|| theater_api_common::load_json("settings.json")).clone()
}

pub fn save_settings(s: Settings) {
    theater_api_common::save_json("settings.json", &s);
    *SETTINGS.lock() = Some(s);
    restart();
}

fn load() {
    theater_api_common::start_relay(None);
    if let Some(obs) = obs_link::obs() {
        obs.add_proc(c"void theater_api_kofi_state(out string json)", state_proc);
        obs.add_proc(c"void theater_api_kofi_recent(out string json)", recent_proc);
    }
    theater_api_common::window::set_ui(ui::ui);
    restart();
}

fn unload() {
    if let Some(t) = TASK.lock().take() {
        t.abort();
    }
}

pub fn restart() {
    if let Some(t) = TASK.lock().take() {
        t.abort();
    }
    let s = settings();
    let polling = !s.verification_token.trim().is_empty();
    with(|st| {
        st.polling = polling;
        st.last_error.clear();
    });
    if polling {
        *TASK.lock() = Some(theater_api_common::runtime().spawn(poll_loop(s)));
    }
    theater_api_common::window::repaint();
}

pub fn status() -> (bool, String, u64) {
    with(|s| (s.polling, s.last_error.clone(), s.events))
}

pub fn recent() -> Vec<Value> {
    with(|s| s.recent.iter().cloned().collect())
}

async fn poll_loop(s: Settings) {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().unwrap_or_default();
    let token = s.verification_token.trim().to_string();
    let page = s.page();
    let url = s.url();
    theater_api_common::log(&format!("☕ Checking Ko-fi every {} s", s.poll_seconds.max(3)));
    loop {
        let sep = if url.contains('?') { '&' } else { '?' };
        let ask = format!("{url}{sep}action=poll&token={}&t={}", token, theater_api_common::unix_secs());
        match client.get(&ask).send().await {
            Ok(resp) => match resp.text().await.map(|t| serde_json::from_str::<Value>(&t)) {
                Ok(Ok(v)) => {
                    with(|st| st.last_error.clear());
                    let items = match &v {
                        Value::Array(a) => a.clone(),
                        _ => v.get("events").and_then(Value::as_array).cloned().unwrap_or_default(),
                    };
                    for item in items {
                        handle(&events::unwrap(&item), &token, &page);
                    }
                }
                _ => with(|st| st.last_error = "Ko-fi's webhook relay answered something unexpected: try again later".into()),
            },
            Err(e) => with(|st| st.last_error = format!("couldn't reach Ko-fi's webhook relay: {e}")),
        }
        theater_api_common::window::repaint();
        tokio::time::sleep(Duration::from_secs(s.poll_seconds.max(3))).await;
    }
}

/// One event from the proxy: checked, then sent on
fn handle(e: &Value, token: &str, page: &str) {
    let test = events::is_kofi_test(e);
    let their_token = e.get("verification_token").and_then(Value::as_str).unwrap_or("").trim();
    if !test && !their_token.is_empty() && their_token != token && their_token != "YOUR_TOKEN" {
        theater_api_common::log("⚠️ Ignored a Ko-fi event with another verification token");
        return;
    }
    let id = events::id(e);
    let duplicate = with(|st| {
        if !test && !id.is_empty() {
            if st.seen.contains(&id) {
                return true;
            }
            st.seen.push_back(id.clone());
            if st.seen.len() > 500 {
                st.seen.pop_front();
            }
        }
        st.events += 1;
        false
    });
    if !duplicate {
        send(e, page, test);
    }
}

fn send(e: &Value, page: &str, test: bool) {
    let messages = events::messages(e, page, test);
    if let Some(a) = messages.first().and_then(|m| m.get("alert")).cloned() {
        with(|st| {
            st.recent.push_front(json!({ "user": a["user"], "amount": a["amount"], "currency": a["currency"], "type": a["kind"], "message": a["message"] }));
            st.recent.truncate(50);
        });
    }
    for m in messages {
        theater_shared::relay::send(&m.to_string());
    }
}

/// The window's Test buttons
pub fn test(kind: &str) {
    send(&events::sample(kind), &settings().page(), true);
}

unsafe extern "C" fn state_proc(_data: *mut c_void, cd: *mut Calldata) {
    let Some(obs) = obs_link::obs() else { return };
    let text = std::panic::catch_unwind(|| {
        let (polling, last_error, events) = status();
        json!({ "polling": polling, "page": settings().page(), "last_error": last_error, "events": events }).to_string()
    })
    .unwrap_or_default();
    obs.set_string(cd, c"json", &text);
}

unsafe extern "C" fn recent_proc(_data: *mut c_void, cd: *mut Calldata) {
    let Some(obs) = obs_link::obs() else { return };
    let text = std::panic::catch_unwind(|| Value::Array(recent()).to_string()).unwrap_or_default();
    obs.set_string(cd, c"json", &text);
}
