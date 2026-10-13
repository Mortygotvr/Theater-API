//! Theater API · Twitch (theater-api-twitch.dll): Twitch's own API, logged in
//! with the twitch_api module's app, in OBS. Its events (redeems with their
//! real titles, cheers, subs, gifts, raids, follows, hype trains, bans, the
//! stream going live) go out through the relay with Theater Reader's
//! triggers and the shared alert, with or without Theater Reader; and the
//! channel's lists are there for the rest of Theater.
//!
//! On OBS's global handlers:
//! - `theater_api_twitch_state(out string json)`: `{"logged_in","login","name","live","refused":[…]}`
//! - `theater_api_twitch_rewards(out string json)`: the rewards, `[{"id","title","cost","image","color","enabled","paused","manageable"}]`
//! - `theater_api_twitch_subscribers(out string json)`: `[{"user","login","tier","gift","gifter"}]`
//! - `theater_api_twitch_followers(out string json)`: `[{"user","login","since"}]`
//! - `theater_api_twitch_channels(out string json)`: the channels whose alerts it sends (`["mortygotvr"]`):
//!   Theater Reader leaves those channels' Twitch alerts to it
//! - `theater_api_twitch_send(in string message, out bool ok)`: a chat message, as the logged-in account
//! - signal `theater_api_twitch_rewards_changed(string json)`: the rewards, when they change

pub mod api;
pub mod events;
pub mod live;
mod ui;

use std::ffi::c_void;

use serde_json::{Value, json};
use theater_api_common::{Info, obs_plugin};
use theater_shared::obs_link::{self, Calldata};

pub static INFO: Info = Info {
    id: "theater-api-twitch",
    name: "Theater API · Twitch",
    name_c: c"Theater API · Twitch",
    description_c: c"Twitch's events (redeems, subs, cheers, raids, follows…) and lists for Theater",
    menu_title: c"\u{1F7E3} Twitch API\u{2026}",
    window_title: "🟣 Twitch API — Theater",
    icon_png: include_bytes!("../../assets/twitch.png"),
};

obs_plugin!(INFO, load: load, unload: unload);

fn load() {
    theater_api_common::start_relay(None);
    register();
    theater_api_common::window::set_ui(ui::ui);
    api::load();
    live::start();
}

fn unload() {
    live::stop();
}

pub fn tier_name(tier: &str) -> String {
    match tier {
        "1000" => "Tier 1",
        "2000" => "Tier 2",
        "3000" => "Tier 3",
        _ => "",
    }
    .to_string()
}

/// The window's Log in: the browser, then live
pub fn log_in() {
    theater_api_common::runtime().spawn(async {
        ui::set_note("Waiting for you to authorize in your browser…");
        match theater_api_common::oauth::login(&api::login_spec()).await {
            Ok(token) => {
                api::set_token(token);
                ui::set_note("");
                live::start();
            }
            Err(e) => {
                theater_api_common::log(&format!("⚠️ Twitch login: {e}"));
                ui::set_note(&format!("Not logged in: {e}"));
            }
        }
    });
}

pub fn log_out() {
    live::stop();
    api::forget();
    ui::set_note("Logged out.");
}

/// A test of `topic`, as if Twitch sent it (marked as a test)
pub fn test(topic: &str) {
    let channel = api::account().map(|a| a.login).unwrap_or_else(|| "testserver".into());
    let rewards = live::rewards();
    let reward = rewards.first().map(|r| (r.id.as_str(), r.title.as_str(), r.cost));
    let event = events::sample(topic, reward);
    live::notification(topic, &event, &channel, &format!("test-{}", theater_api_common::unix_secs()), true);
}

// ---------------------------------------------------------------------------
// OBS procedures

fn register() {
    let Some(obs) = obs_link::obs() else { return };
    obs.add_signal(c"void theater_api_twitch_rewards_changed(string json)");
    obs.add_proc(c"void theater_api_twitch_state(out string json)", state_proc);
    obs.add_proc(c"void theater_api_twitch_rewards(out string json)", rewards_proc);
    obs.add_proc(c"void theater_api_twitch_subscribers(out string json)", subscribers_proc);
    obs.add_proc(c"void theater_api_twitch_followers(out string json)", followers_proc);
    obs.add_proc(c"void theater_api_twitch_channels(out string json)", channels_proc);
    obs.add_proc(c"void theater_api_twitch_send(in string message, out bool ok)", send_proc);
}

/// The rewards changed (or were loaded): the signal, and the window
pub fn rewards_changed() {
    if let Some(obs) = obs_link::obs() {
        obs.signal_string(c"theater_api_twitch_rewards_changed", c"json", &rewards_json());
    }
    theater_api_common::window::repaint();
}

fn rewards_json() -> String {
    serde_json::to_string(&live::rewards()).unwrap_or_else(|_| "[]".into())
}

pub fn state_json() -> Value {
    let account = api::account();
    json!({
        "logged_in": api::logged_in(),
        "login": account.as_ref().map(|a| a.login.clone()).unwrap_or_default(),
        "name": account.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        "live": live::link() == live::Link::Connected,
        "refused": live::refused().iter().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
    })
}

fn out_string(cd: *mut Calldata, value: impl FnOnce() -> String + std::panic::UnwindSafe) {
    let Some(obs) = obs_link::obs() else { return };
    let text = std::panic::catch_unwind(value).unwrap_or_default();
    obs.set_string(cd, c"json", &text);
}

unsafe extern "C" fn state_proc(_data: *mut c_void, cd: *mut Calldata) {
    out_string(cd, || state_json().to_string());
}

unsafe extern "C" fn rewards_proc(_data: *mut c_void, cd: *mut Calldata) {
    out_string(cd, rewards_json);
}

unsafe extern "C" fn subscribers_proc(_data: *mut c_void, cd: *mut Calldata) {
    out_string(cd, || Value::Array(live::subscribers()).to_string());
}

unsafe extern "C" fn followers_proc(_data: *mut c_void, cd: *mut Calldata) {
    out_string(cd, || Value::Array(live::followers()).to_string());
}

unsafe extern "C" fn channels_proc(_data: *mut c_void, cd: *mut Calldata) {
    out_string(cd, || {
        let live = live::link() == live::Link::Connected;
        let channels: Vec<String> = api::account().filter(|_| live).map(|a| vec![a.login]).unwrap_or_default();
        json!(channels).to_string()
    });
}

unsafe extern "C" fn send_proc(_data: *mut c_void, cd: *mut Calldata) {
    let Some(obs) = obs_link::obs() else { return };
    let message = obs.get_string(cd, c"message");
    let ok = !message.trim().is_empty() && api::logged_in();
    if ok {
        theater_api_common::runtime().spawn(async move {
            if let Err(e) = api::send_chat(&message).await {
                theater_api_common::log(&format!("⚠️ Couldn't send the chat message: {e}"));
            }
        });
    }
    obs.set_bool(cd, c"ok", ok);
}
