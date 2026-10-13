//! Twitch's EventSub notifications as Theater's messages: the alert (with
//! the shared alert, theater_shared::alert) and the trigger feed
//! ("moderation", for the Control Panel's triggers), as Theater Reader sends
//! them, with the same triggers ("Twitch mortygotvr redeem Hydrate"), so
//! triggers made for Theater Reader work the same.

use serde_json::{Map, Value, json};
use theater_shared::alert::Alert;

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn number(v: &Value, key: &str) -> f64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn tier_name(tier: &str) -> String {
    match tier {
        "1000" => "Tier 1",
        "2000" => "Tier 2",
        "3000" => "Tier 3",
        _ => "",
    }
    .to_string()
}

/// A reward's picture, from the reward list (EventSub's redemption has none)
pub type RewardImage<'a> = &'a dyn Fn(&str) -> String;

/// The messages for one notification (`kind`: its subscription type), or
/// none for what Theater shows nothing for (each gifted sub's own sub event:
/// the gift's event covers them).
pub fn messages(kind: &str, event: &Value, channel: &str, msg_id: &str, test: bool, reward_image: RewardImage) -> Vec<Value> {
    let user = {
        let name = text(event, "user_name");
        if name.is_empty() { text(event, "user_login") } else { name }
    };
    let mut alert = Alert { platform: "twitch".into(), channel: channel.to_string(), user: user.clone(), test, raw: event.clone(), ..Default::default() };
    // Reader's data keys too, for what reads them ({username}, {message}…)
    let mut data = Map::new();
    data.insert("username".into(), Value::from(user.clone()));
    data.insert("display_name".into(), Value::from(user.clone()));
    let words: String;
    let event_name: &str;
    match kind {
        "channel.channel_points_custom_reward_redemption.add" => {
            let reward = event.get("reward").cloned().unwrap_or(Value::Null);
            let title = text(&reward, "title");
            alert.kind = "redeem".into();
            alert.title = title.clone();
            alert.reward_id = text(&reward, "id");
            alert.cost = number(&reward, "cost");
            alert.message = text(event, "user_input");
            alert.image = reward_image(&alert.reward_id);
            data.insert("reward_title".into(), Value::from(title.clone()));
            data.insert("reward_id".into(), Value::from(alert.reward_id.clone()));
            data.insert("cost".into(), Value::from(alert.cost));
            data.insert("message".into(), Value::from(alert.message.clone()));
            data.insert("user_input".into(), Value::from(alert.message.clone()));
            data.insert("image".into(), Value::from(alert.image.clone()));
            event_name = "Twitch redeem";
            words = format!("redeem {title}");
        }
        "channel.cheer" => {
            if event.get("is_anonymous").and_then(Value::as_bool) == Some(true) {
                alert.user = "Anonymous".into();
                alert.anonymous = true;
            }
            alert.kind = "cheer".into();
            alert.amount = number(event, "bits");
            alert.message = text(event, "message");
            data.insert("bits".into(), Value::from(alert.amount));
            data.insert("message".into(), Value::from(alert.message.clone()));
            event_name = "Twitch cheer";
            words = format!("cheer {} bits", alert.amount as i64);
        }
        "channel.subscribe" => {
            if event.get("is_gift").and_then(Value::as_bool) == Some(true) {
                return Vec::new();
            }
            alert.kind = "sub".into();
            alert.tier = tier_name(&text(event, "tier"));
            data.insert("type".into(), Value::from("sub"));
            data.insert("plan".into(), Value::from(text(event, "tier")));
            event_name = "Twitch sub";
            words = "sub".into();
        }
        "channel.subscription.message" => {
            alert.kind = "resub".into();
            alert.tier = tier_name(&text(event, "tier"));
            alert.months = number(event, "cumulative_months");
            alert.message = event.get("message").map(|m| text(m, "text")).unwrap_or_default();
            data.insert("type".into(), Value::from("resub"));
            data.insert("plan".into(), Value::from(text(event, "tier")));
            data.insert("months".into(), Value::from(alert.months));
            data.insert("message".into(), Value::from(alert.message.clone()));
            event_name = "Twitch sub";
            words = "sub".into();
        }
        "channel.subscription.gift" => {
            alert.kind = "gift".into();
            alert.anonymous = event.get("is_anonymous").and_then(Value::as_bool) == Some(true);
            if alert.anonymous || alert.user.is_empty() {
                alert.user = "Anonymous".into();
            }
            alert.amount = number(event, "total").max(1.0);
            alert.tier = tier_name(&text(event, "tier"));
            data.insert("type".into(), Value::from(if alert.anonymous { "anonsubgift" } else { "subgift" }));
            data.insert("plan".into(), Value::from(text(event, "tier")));
            data.insert("count".into(), Value::from(alert.amount));
            event_name = "Twitch sub";
            words = "sub".into();
        }
        "channel.raid" => {
            alert.kind = "raid".into();
            alert.user = text(event, "from_broadcaster_user_name");
            alert.amount = number(event, "viewers");
            data.insert("username".into(), Value::from(alert.user.clone()));
            data.insert("viewers".into(), Value::from(alert.amount));
            event_name = "Twitch raid";
            words = "raid".into();
        }
        "channel.follow" => {
            alert.kind = "follow".into();
            event_name = "Twitch follow";
            words = "follow".into();
        }
        "channel.hype_train.begin" | "channel.hype_train.progress" => {
            alert.kind = "hype_train".into();
            alert.user.clear();
            alert.amount = number(event, "level").max(1.0);
            alert.progress = number(event, "progress");
            alert.goal = number(event, "goal");
            data.insert("level".into(), Value::from(alert.amount));
            data.insert("progress".into(), Value::from(alert.progress));
            data.insert("goal".into(), Value::from(alert.goal));
            event_name = "Twitch hype train";
            words = format!("hype train {}", alert.amount as i64);
        }
        "channel.ban" => {
            let permanent = event.get("is_permanent").and_then(Value::as_bool).unwrap_or(true);
            if permanent {
                alert.kind = "ban".into();
                event_name = "Twitch ban";
                words = "ban".into();
            } else {
                alert.kind = "timeout".into();
                alert.duration = duration_secs(&text(event, "banned_at"), &text(event, "ends_at"));
                data.insert("duration".into(), Value::from(alert.duration));
                event_name = "Twitch timeout";
                words = "timeout".into();
            }
        }
        "stream.online" => {
            alert.kind = "stream_start".into();
            alert.user.clear();
            event_name = "Twitch stream start";
            words = "stream start".into();
        }
        "stream.offline" => {
            alert.kind = "stream_end".into();
            alert.user.clear();
            event_name = "Twitch stream end";
            words = "stream end".into();
        }
        _ => return Vec::new(),
    }
    if test {
        data.insert("is_test".into(), Value::from(true));
    }
    let trigger = format!("Twitch {channel} {words}");
    alert.trigger = trigger.clone();
    let mut raw = event.as_object().cloned().unwrap_or_default();
    raw.extend(data.clone());
    vec![
        json!({
            "type": "alert",
            "source": "TwitchAPI",
            "event": event_name,
            "trigger": trigger,
            "msg_id": msg_id,
            "data": Value::Object(raw.clone()),
            "alert": alert.to_value(),
        }),
        json!({
            "type": "moderation",
            "payload": {
                "trigger": trigger,
                "msg_id": msg_id,
                "platform": "twitch",
                "username": alert.user,
                "message": alert.message,
                "customData": Value::Object(raw),
                "is_suspicious": false,
                "chat_moderate_enabled": false,
            }
        }),
    ]
}

/// Seconds between two RFC 3339 times ("2026-10-13T20:00:00Z"), 0 when unreadable
fn duration_secs(from: &str, to: &str) -> f64 {
    match (seconds_of(from), seconds_of(to)) {
        (Some(a), Some(b)) if b > a => (b - a) as f64,
        _ => 0.0,
    }
}

/// An RFC 3339 time as seconds since 2000 (enough for a difference)
fn seconds_of(t: &str) -> Option<i64> {
    let t = t.get(..19)?;
    let n = |r: std::ops::Range<usize>| t.get(r).and_then(|s| s.parse::<i64>().ok());
    let (y, mo, d, h, mi, s) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let days_before_month = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mut days = (2000..y).map(|y| if leap(y) { 366 } else { 365 }).sum::<i64>() + days_before_month[(mo - 1).clamp(0, 11) as usize] + d - 1;
    if mo > 2 && leap(y) {
        days += 1;
    }
    Some(((days * 24 + h) * 60 + mi) * 60 + s)
}

/// A test notification of `kind` (the window's Test buttons), as Twitch sends them
pub fn sample(kind: &str, reward: Option<(&str, &str, f64)>) -> Value {
    match kind {
        "channel.channel_points_custom_reward_redemption.add" => {
            let (id, title, cost) = reward.unwrap_or(("test-reward", "Hydrate!", 500.0));
            json!({ "user_name": "TestViewer", "user_login": "testviewer", "user_input": "", "reward": { "id": id, "title": title, "cost": cost } })
        }
        "channel.cheer" => json!({ "user_name": "TestViewer", "bits": 500, "message": "Cheer500 Take my bits!", "is_anonymous": false }),
        "channel.subscription.message" => json!({ "user_name": "TestViewer", "tier": "1000", "cumulative_months": 6, "message": { "text": "Six months already!" } }),
        "channel.subscription.gift" => json!({ "user_name": "TestGifter", "tier": "1000", "total": 5, "is_anonymous": false }),
        "channel.raid" => json!({ "from_broadcaster_user_name": "TestRaider", "viewers": 42 }),
        "channel.follow" => json!({ "user_name": "TestFollower" }),
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_image(_: &str) -> String {
        String::new()
    }

    #[test]
    fn a_redeem_has_readers_trigger_its_title_cost_and_the_rewards_picture() {
        let event = json!({ "user_name": "Ann", "user_login": "ann", "user_input": "hi", "reward": { "id": "r1", "title": "Hydrate!", "cost": 1500 } });
        let image = |id: &str| if id == "r1" { "https://x/4.png".to_string() } else { String::new() };
        let out = messages("channel.channel_points_custom_reward_redemption.add", &event, "mortygotvr", "m1", false, &image);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["trigger"], "Twitch mortygotvr redeem Hydrate!");
        let a = &out[0]["alert"];
        assert_eq!((a["kind"].as_str(), a["title"].as_str(), a["reward_id"].as_str(), a["cost"].as_f64()), (Some("redeem"), Some("Hydrate!"), Some("r1"), Some(1500.0)));
        assert_eq!((a["user"].as_str(), a["message"].as_str(), a["image"].as_str()), (Some("Ann"), Some("hi"), Some("https://x/4.png")));
        // What Reader's readers expect
        assert_eq!(out[0]["data"]["reward_title"], "Hydrate!");
        assert_eq!(out[0]["data"]["username"], "Ann");
        assert_eq!(out[1]["type"], "moderation");
        assert_eq!(out[1]["payload"]["trigger"], "Twitch mortygotvr redeem Hydrate!");
        // The shared alert, read back by Theater Scene, is the same
        assert_eq!(theater_shared::alert::from_relay(&out[0]).unwrap().title, "Hydrate!");
    }

    #[test]
    fn cheers_subs_gifts_raids_and_follows() {
        let t = |kind: &str, e: Value| messages(kind, &e, "m", "id", false, &no_image);
        let cheer = t("channel.cheer", json!({ "user_name": "K", "bits": 500, "message": "x", "is_anonymous": false }));
        assert_eq!(cheer[0]["trigger"], "Twitch m cheer 500 bits");
        assert_eq!(cheer[0]["alert"]["amount"], 500.0);
        let resub = t("channel.subscription.message", json!({ "user_name": "A", "tier": "2000", "cumulative_months": 14, "message": { "text": "yo" } }));
        assert_eq!(resub[0]["trigger"], "Twitch m sub");
        assert_eq!((resub[0]["alert"]["kind"].as_str(), resub[0]["alert"]["tier"].as_str(), resub[0]["alert"]["months"].as_f64()), (Some("resub"), Some("Tier 2"), Some(14.0)));
        assert!(t("channel.subscribe", json!({ "user_name": "R", "tier": "1000", "is_gift": true })).is_empty(), "the gift's event covers it");
        let gift = t("channel.subscription.gift", json!({ "user_name": null, "tier": "1000", "total": 5, "is_anonymous": true }));
        assert_eq!((gift[0]["alert"]["user"].as_str(), gift[0]["alert"]["amount"].as_f64(), gift[0]["alert"]["anonymous"].as_bool()), (Some("Anonymous"), Some(5.0), Some(true)));
        let raid = t("channel.raid", json!({ "from_broadcaster_user_name": "Big", "viewers": 128 }));
        assert_eq!((raid[0]["trigger"].as_str(), raid[0]["alert"]["user"].as_str(), raid[0]["alert"]["amount"].as_f64()), (Some("Twitch m raid"), Some("Big"), Some(128.0)));
        assert_eq!(t("channel.follow", json!({ "user_name": "F" }))[0]["trigger"], "Twitch m follow");
    }

    #[test]
    fn hype_trains_bans_timeouts_and_the_stream() {
        let t = |kind: &str, e: Value| messages(kind, &e, "m", "id", false, &no_image);
        let hype = t("channel.hype_train.progress", json!({ "level": 3, "progress": 250, "goal": 1000 }));
        assert_eq!(hype[0]["trigger"], "Twitch m hype train 3");
        assert_eq!(hype[0]["alert"]["user"], "");
        let to = t("channel.ban", json!({ "user_name": "T", "is_permanent": false, "banned_at": "2026-10-13T20:00:00Z", "ends_at": "2026-10-13T20:10:00Z" }));
        assert_eq!((to[0]["trigger"].as_str(), to[0]["alert"]["duration"].as_f64()), (Some("Twitch m timeout"), Some(600.0)));
        assert_eq!(t("channel.ban", json!({ "user_name": "T", "is_permanent": true }))[0]["trigger"], "Twitch m ban");
        assert_eq!(t("stream.online", json!({}))[0]["trigger"], "Twitch m stream start");
        assert!(t("channel.something.new", json!({})).is_empty());
        assert_eq!(seconds_of("2024-03-01T00:00:00Z").unwrap() - seconds_of("2024-02-28T00:00:00Z").unwrap(), 2 * 86400, "a leap year");
    }

    #[test]
    fn test_notifications_are_marked() {
        let out = messages("channel.follow", &sample("channel.follow", None), "m", "t", true, &no_image);
        assert_eq!((out[0]["alert"]["test"].as_bool(), out[0]["data"]["is_test"].as_bool()), (Some(true), Some(true)));
    }
}
