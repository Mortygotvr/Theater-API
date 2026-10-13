//! Twitch live: EventSub over its websocket (redeems, cheers, subs, raids,
//! follows, hype trains, bans, the stream going live), sent on through the
//! relay; and the lists kept up to date: rewards, subscribers, followers.

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use crate::api;

const EVENTSUB: &str = "wss://eventsub.wss.twitch.tv/ws";

/// What it subscribes to: (type, version, condition key for the channel's id)
const TOPICS: [(&str, &str); 13] = [
    ("channel.channel_points_custom_reward_redemption.add", "1"),
    ("channel.cheer", "1"),
    ("channel.subscribe", "1"),
    ("channel.subscription.message", "1"),
    ("channel.subscription.gift", "1"),
    ("channel.raid", "1"),
    ("channel.follow", "2"),
    ("channel.hype_train.begin", "2"),
    ("channel.hype_train.progress", "2"),
    ("channel.ban", "1"),
    ("stream.online", "1"),
    ("stream.offline", "1"),
    // The reward list changing
    ("channel.channel_points_custom_reward.update", "1"),
];
const REWARD_TOPICS: [&str; 2] = ["channel.channel_points_custom_reward.add", "channel.channel_points_custom_reward.remove"];

fn condition(kind: &str, user_id: &str) -> Value {
    match kind {
        "channel.raid" => json!({ "to_broadcaster_user_id": user_id }),
        "channel.follow" => json!({ "broadcaster_user_id": user_id, "moderator_user_id": user_id }),
        _ => json!({ "broadcaster_user_id": user_id }),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Off,
    Connecting,
    Connected,
    Failed(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct Reward {
    pub id: String,
    pub title: String,
    pub cost: f64,
    pub image: String,
    pub color: String,
    pub enabled: bool,
    pub paused: bool,
    /// Made by this app (Theater): it may pause, edit and refund it
    pub manageable: bool,
}

#[derive(Default)]
struct State {
    link: Option<Link>,
    /// Topics Twitch refused, and why
    refused: Vec<(String, String)>,
    rewards: Vec<Reward>,
    rewards_note: String,
    subscribers: Vec<Value>,
    followers: Vec<Value>,
    /// Message ids seen lately: Twitch may send one twice
    seen: VecDeque<String>,
    events: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static TASK: Mutex<Option<tokio::task::JoinHandle<()>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut s = STATE.lock();
    f(s.get_or_insert_with(State::default))
}

pub fn link() -> Link {
    with(|s| s.link.clone().unwrap_or(Link::Off))
}
pub fn refused() -> Vec<(String, String)> {
    with(|s| s.refused.clone())
}
pub fn rewards() -> Vec<Reward> {
    with(|s| s.rewards.clone())
}
pub fn rewards_note() -> String {
    with(|s| s.rewards_note.clone())
}
pub fn subscribers() -> Vec<Value> {
    with(|s| s.subscribers.clone())
}
pub fn followers() -> Vec<Value> {
    with(|s| s.followers.clone())
}
pub fn events_count() -> u64 {
    with(|s| s.events)
}

fn set_link(link: Link) {
    with(|s| s.link = Some(link));
    theater_api_common::window::repaint();
}

/// (Re)starts: checks the login, then stays connected.
pub fn start() {
    stop();
    if !api::logged_in() {
        set_link(Link::Off);
        return;
    }
    set_link(Link::Connecting);
    *TASK.lock() = Some(theater_api_common::runtime().spawn(run()));
}

pub fn stop() {
    if let Some(task) = TASK.lock().take() {
        task.abort();
    }
    set_link(Link::Off);
}

async fn run() {
    let account = match api::validate().await {
        Ok(a) => a,
        Err(e) => {
            theater_api_common::log(&format!("⚠️ Twitch login: {e}"));
            set_link(Link::Failed(e));
            return;
        }
    };
    theater_api_common::log(&format!("🟣 Logged in to Twitch as {}", account.login));
    crate::rewards_changed();
    tokio::spawn(refresh_lists());
    let mut url = EVENTSUB.to_string();
    let mut subscribed = false;
    loop {
        match session(&url, &account, subscribed).await {
            // Twitch moves the session: the subscriptions go with it
            Ok(Some(reconnect)) => {
                url = reconnect;
                subscribed = true;
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                theater_api_common::log(&format!("⚠️ EventSub: {e}; reconnecting in 5 s"));
                set_link(Link::Failed(format!("{e} (reconnecting…)")));
            }
        }
        url = EVENTSUB.to_string();
        subscribed = false;
        tokio::time::sleep(Duration::from_secs(5)).await;
        set_link(Link::Connecting);
    }
}

/// One EventSub connection, until it closes (Ok(None)), Twitch asks to move
/// (Ok(Some(url))) or it fails.
async fn session(url: &str, account: &api::Account, subscribed: bool) -> Result<Option<String>, String> {
    let (ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| format!("couldn't connect: {e}"))?;
    let (_write, mut read) = ws.split();
    let mut keepalive = Duration::from_secs(30);
    loop {
        let message = match tokio::time::timeout(keepalive + Duration::from_secs(10), read.next()).await {
            Err(_) => return Err("Twitch went quiet".into()),
            Ok(None) => return Ok(None),
            Ok(Some(Err(e))) => return Err(e.to_string()),
            Ok(Some(Ok(m))) => m,
        };
        let text = match message {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => return Ok(None),
            _ => continue,
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let kind = v.pointer("/metadata/message_type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "session_welcome" => {
                let id = v.pointer("/payload/session/id").and_then(Value::as_str).unwrap_or("").to_string();
                if let Some(secs) = v.pointer("/payload/session/keepalive_timeout_seconds").and_then(Value::as_u64) {
                    keepalive = Duration::from_secs(secs.max(10));
                }
                if !subscribed {
                    subscribe_all(&id, &account.user_id).await;
                }
                set_link(Link::Connected);
            }
            "session_reconnect" => {
                if let Some(next) = v.pointer("/payload/session/reconnect_url").and_then(Value::as_str) {
                    return Ok(Some(next.to_string()));
                }
            }
            "notification" => {
                let id = v.pointer("/metadata/message_id").and_then(Value::as_str).unwrap_or("").to_string();
                let duplicate = with(|s| {
                    if s.seen.contains(&id) {
                        return true;
                    }
                    s.seen.push_back(id.clone());
                    if s.seen.len() > 200 {
                        s.seen.pop_front();
                    }
                    s.events += 1;
                    false
                });
                if duplicate {
                    continue;
                }
                let topic = v.pointer("/payload/subscription/type").and_then(Value::as_str).unwrap_or("");
                let event = v.pointer("/payload/event").cloned().unwrap_or(Value::Null);
                notification(topic, &event, &account.login, &id, false);
            }
            "revocation" => {
                let topic = v.pointer("/payload/subscription/type").and_then(Value::as_str).unwrap_or("?");
                let why = v.pointer("/payload/subscription/status").and_then(Value::as_str).unwrap_or("revoked");
                theater_api_common::log(&format!("⚠️ Twitch stopped sending {topic}: {why}"));
                with(|s| s.refused.push((topic.to_string(), why.to_string())));
            }
            _ => {}
        }
    }
}

async fn subscribe_all(session_id: &str, user_id: &str) {
    with(|s| s.refused.clear());
    for (kind, version) in TOPICS.iter().copied().chain(REWARD_TOPICS.iter().map(|k| (*k, "1"))) {
        if let Err(e) = api::subscribe(session_id, kind, version, condition(kind, user_id)).await {
            // A channel without channel points (not Affiliate/Partner) can't have the redeem topics
            theater_api_common::log(&format!("⚠️ EventSub {kind}: {e}"));
            with(|s| s.refused.push((kind.to_string(), e)));
        }
    }
    theater_api_common::window::repaint();
}

/// A notification (or a test one): the reward list's changes, else messages
/// sent on through the relay.
pub fn notification(topic: &str, event: &Value, channel: &str, msg_id: &str, test: bool) {
    if topic.starts_with("channel.channel_points_custom_reward.") {
        theater_api_common::runtime().spawn(refresh_rewards());
        return;
    }
    let image = |reward_id: &str| -> String { with(|s| s.rewards.iter().find(|r| r.id == reward_id).map(|r| r.image.clone()).unwrap_or_default()) };
    for message in crate::events::messages(topic, event, channel, msg_id, test, &image) {
        theater_shared::relay::send(&message.to_string());
    }
    if matches!(topic, "channel.subscribe" | "channel.subscription.gift" | "channel.follow") && !test {
        theater_api_common::runtime().spawn(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            refresh_lists().await;
        });
    }
}

/// The channel's rewards, and which Theater may manage
pub async fn refresh_rewards() {
    let Some(account) = api::account() else { return };
    let all = api::helix("GET", &format!("/channel_points/custom_rewards?broadcaster_id={}", account.user_id), None).await;
    let mine = api::helix("GET", &format!("/channel_points/custom_rewards?broadcaster_id={}&only_manageable_rewards=true", account.user_id), None).await;
    match all {
        Ok(v) => {
            let manageable: Vec<String> = mine
                .ok()
                .and_then(|m| m.get("data").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_string))
                .collect();
            let rewards: Vec<Reward> = v
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|r| {
                    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let image = r
                        .pointer("/image/url_4x")
                        .or_else(|| r.pointer("/default_image/url_4x"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    Reward {
                        id: s("id"),
                        title: s("title"),
                        cost: r.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
                        image,
                        color: s("background_color"),
                        enabled: r.get("is_enabled").and_then(Value::as_bool).unwrap_or(true),
                        paused: r.get("is_paused").and_then(Value::as_bool).unwrap_or(false),
                        manageable: manageable.contains(&s("id")),
                    }
                })
                .collect();
            with(|s| {
                s.rewards = rewards;
                s.rewards_note.clear();
            });
        }
        Err(e) => {
            let note = if e.starts_with("403") { "No channel points: they need an Affiliate or Partner channel.".to_string() } else { format!("Couldn't load the rewards: {e}") };
            with(|s| s.rewards_note = note);
        }
    }
    crate::rewards_changed();
}

/// The subscriber and follower lists (and the rewards)
pub async fn refresh_lists() {
    let Some(account) = api::account() else { return };
    refresh_rewards().await;
    if let Ok(subs) = api::helix_all(&format!("/subscriptions?broadcaster_id={}", account.user_id), 5000).await {
        let subs = subs
            .iter()
            .filter(|s| s.get("user_id").and_then(Value::as_str) != Some(account.user_id.as_str()))
            .map(|s| {
                let t = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                json!({ "user": t("user_name"), "login": t("user_login"), "tier": crate::tier_name(&t("tier")), "gift": s.get("is_gift").and_then(Value::as_bool).unwrap_or(false), "gifter": t("gifter_name") })
            })
            .collect();
        with(|s| s.subscribers = subs);
    }
    if let Ok(follows) = api::helix_all(&format!("/channels/followers?broadcaster_id={}", account.user_id), 5000).await {
        let follows = follows
            .iter()
            .map(|f| {
                let t = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                json!({ "user": t("user_name"), "login": t("user_login"), "since": t("followed_at") })
            })
            .collect();
        with(|s| s.followers = follows);
    }
    theater_api_common::window::repaint();
}
