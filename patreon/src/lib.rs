//! Theater API · Patreon (theater-api-patreon.dll): your campaign's patrons,
//! logged in with the patreon module's app (through its proxy). It looks at
//! the members regularly and compares, as the module does, and sends new,
//! changed and ended pledges out through the relay with the shared alert.
//! Triggers: "Patreon <name> pledge <tier>", "Patreon <name> pledge change",
//! "Patreon <name> pledge end".
//!
//! On OBS's global handlers:
//! - `theater_api_patreon_state(out string json)`: `{"logged_in","campaign","patrons","last_error"}`
//! - `theater_api_patreon_members(out string json)`: `[{"id","name","status","cents","tier","lifetime_cents","since"}]`

pub mod events;
mod ui;

use std::collections::HashMap;
use std::ffi::c_void;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use theater_api_common::oauth::{self, Login, Token};
use theater_api_common::{Info, obs_plugin};
use theater_shared::obs_link::{self, Calldata};

use events::Member;

pub static INFO: Info = Info {
    id: "theater-api-patreon",
    name: "Theater API · Patreon",
    name_c: c"Theater API · Patreon",
    description_c: c"Patreon pledges and patrons for Theater",
    menu_title: c"\u{1F9E1} Patreon API\u{2026}",
    window_title: "🧡 Patreon API — Theater",
    icon_png: include_bytes!("../../assets/patreon.png"),
};

obs_plugin!(INFO, load: load, unload: unload);

/// The patreon module's app (patreon.tcp)
const CLIENT_ID: &str = "ukd3366dQp93ZbciK9kI3fDXd0JniEpxxiZsGAMXlmSuMXbrGxhLYF4x65m87eR4";
const API: &str = "https://www.patreon.com/api/oauth2/v2";

pub fn login_spec() -> Login {
    Login { authorize: "https://www.patreon.com/oauth2/authorize", client_id: CLIENT_ID, scopes: vec!["identity", "campaigns", "campaigns.members"], proxy: theater_api_common::proxies::PATREON }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The trigger's channel: "Patreon <name> pledge"
    pub name: String,
    pub poll_seconds: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { name: "patreon".into(), poll_seconds: 60 }
    }
}

impl Settings {
    pub fn name(&self) -> String {
        let n: String = self.name.split_whitespace().collect::<Vec<_>>().join("");
        if n.is_empty() { "patreon".into() } else { n }
    }
}

#[derive(Default)]
struct State {
    token: Option<Token>,
    campaign: String,
    campaign_id: String,
    members: Vec<Member>,
    last_error: String,
    note: String,
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

pub fn logged_in() -> bool {
    with(|s| s.token.as_ref().is_some_and(|t| !t.is_empty()))
}

/// (campaign, patrons active, members, last error, note)
pub fn status() -> (String, usize, usize, String, String) {
    with(|s| (s.campaign.clone(), s.members.iter().filter(|m| m.active()).count(), s.members.len(), s.last_error.clone(), s.note.clone()))
}

pub fn members() -> Vec<Member> {
    with(|s| s.members.clone())
}

pub fn set_note(note: &str) {
    with(|s| s.note = note.to_string());
    theater_api_common::window::repaint();
}

fn load() {
    theater_api_common::start_relay(None);
    if let Some(obs) = obs_link::obs() {
        obs.add_proc(c"void theater_api_patreon_state(out string json)", state_proc);
        obs.add_proc(c"void theater_api_patreon_members(out string json)", members_proc);
    }
    theater_api_common::window::set_ui(ui::ui);
    let token = oauth::load_token();
    if !token.is_empty() {
        with(|s| s.token = Some(token));
    }
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
    if logged_in() {
        *TASK.lock() = Some(theater_api_common::runtime().spawn(poll_loop(settings())));
    }
    theater_api_common::window::repaint();
}

pub fn log_in() {
    theater_api_common::runtime().spawn(async {
        set_note("Waiting for you to authorize in your browser…");
        match oauth::login(&login_spec()).await {
            Ok(token) => {
                with(|s| s.token = Some(token));
                set_note("");
                restart();
            }
            Err(e) => {
                theater_api_common::log(&format!("⚠️ Patreon login: {e}"));
                set_note(&format!("Not logged in: {e}"));
            }
        }
    });
}

pub fn log_out() {
    if let Some(t) = TASK.lock().take() {
        t.abort();
    }
    oauth::forget_token();
    with(|s| *s = State { note: "Logged out.".into(), ..State::default() });
    theater_api_common::window::repaint();
}

/// A Patreon API call, refreshing the token once (through the proxy) on 401
async fn patreon(url: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    for attempt in 0..2 {
        let token = with(|s| s.token.clone()).ok_or("not logged in")?;
        if token.stale() && attempt == 0 {
            refresh(&token).await?;
            continue;
        }
        let resp = client.get(url).bearer_auth(&token.access_token).send().await.map_err(|e| format!("couldn't reach Patreon: {e}"))?;
        let status = resp.status().as_u16();
        if status == 401 && attempt == 0 {
            refresh(&token).await?;
            continue;
        }
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(v);
        }
        let why = v.pointer("/errors/0/detail").and_then(Value::as_str).unwrap_or("refused");
        return Err(format!("{status}: {why}"));
    }
    Err("Patreon refused the login: log in again".into())
}

async fn refresh(token: &Token) -> Result<(), String> {
    match oauth::refresh(theater_api_common::proxies::PATREON, token).await {
        Ok(new) => {
            with(|s| s.token = Some(new));
            Ok(())
        }
        Err(e) => Err(format!("the login expired and couldn't be renewed ({e}): log in again")),
    }
}

/// The campaign, then every page of its members
async fn look() -> Result<Vec<Member>, String> {
    if with(|s| s.campaign_id.is_empty()) {
        let v = patreon(&format!("{API}/campaigns?fields%5Bcampaign%5D=creation_name,vanity,patron_count")).await?;
        let id = v.pointer("/data/0/id").and_then(Value::as_str).ok_or("this Patreon account has no campaign")?.to_string();
        let name = v
            .pointer("/data/0/attributes/creation_name")
            .or_else(|| v.pointer("/data/0/attributes/vanity"))
            .and_then(Value::as_str)
            .unwrap_or("your campaign")
            .to_string();
        with(|s| {
            s.campaign_id = id;
            s.campaign = name;
        });
    }
    let id = with(|s| s.campaign_id.clone());
    let mut url = format!(
        "{API}/campaigns/{id}/members?include=currently_entitled_tiers&fields%5Bmember%5D=full_name,patron_status,currently_entitled_amount_cents,lifetime_support_cents,pledge_relationship_start&fields%5Btier%5D=title&page%5Bcount%5D=500"
    );
    let mut all = Vec::new();
    for _ in 0..50 {
        let page = patreon(&url).await?;
        all.extend(events::members_of(&page));
        match page.pointer("/links/next").and_then(Value::as_str) {
            Some(next) if !next.is_empty() => url = next.to_string(),
            _ => break,
        }
    }
    Ok(all)
}

async fn poll_loop(s: Settings) {
    let mut first = true;
    loop {
        match look().await {
            Ok(now) => {
                let before: HashMap<String, Member> = with(|st| st.members.iter().map(|m| (m.id.clone(), m.clone())).collect());
                // The first look only learns who's there
                if !first {
                    for (what, m, before_cents) in events::changes(&before, &now) {
                        for message in events::messages(what, &m, before_cents, &s.name(), false) {
                            theater_shared::relay::send(&message.to_string());
                        }
                    }
                }
                first = false;
                with(|st| {
                    st.members = now;
                    st.last_error.clear();
                });
            }
            Err(e) => {
                theater_api_common::log(&format!("⚠️ Patreon: {e}"));
                with(|st| st.last_error = e);
            }
        }
        theater_api_common::window::repaint();
        tokio::time::sleep(Duration::from_secs(s.poll_seconds.max(30))).await;
    }
}

/// The window's Test buttons
pub fn test(what: &str) {
    let m = Member { id: "test".into(), name: "TestPatron".into(), status: "active_patron".into(), cents: 500, tier: "Gold".into(), lifetime_cents: 500, since: String::new() };
    for message in events::messages(what, &m, 300, &settings().name(), true) {
        theater_shared::relay::send(&message.to_string());
    }
}

unsafe extern "C" fn state_proc(_data: *mut c_void, cd: *mut Calldata) {
    let Some(obs) = obs_link::obs() else { return };
    let text = std::panic::catch_unwind(|| {
        let (campaign, patrons, _, last_error, _) = status();
        json!({ "logged_in": logged_in(), "campaign": campaign, "patrons": patrons, "last_error": last_error }).to_string()
    })
    .unwrap_or_default();
    obs.set_string(cd, c"json", &text);
}

unsafe extern "C" fn members_proc(_data: *mut c_void, cd: *mut Calldata) {
    let Some(obs) = obs_link::obs() else { return };
    let text = std::panic::catch_unwind(|| serde_json::to_string(&members()).unwrap_or_default()).unwrap_or_default();
    obs.set_string(cd, c"json", &text);
}
