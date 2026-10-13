//! Patreon's members, compared from one look to the next (as the patreon
//! module does), as Theater's messages: a new pledge, a changed one, an
//! ended one. Triggers: "Patreon <name> pledge <tier>",
//! "Patreon <name> pledge change", "Patreon <name> pledge end".

use std::collections::HashMap;

use serde::Serialize;
use serde_json::{Value, json};
use theater_shared::alert::Alert;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Member {
    pub id: String,
    pub name: String,
    /// active_patron, declined_patron, former_patron
    pub status: String,
    pub cents: i64,
    pub tier: String,
    pub lifetime_cents: i64,
    pub since: String,
}

impl Member {
    pub fn active(&self) -> bool {
        self.status == "active_patron"
    }
}

/// The members of one page of Patreon's members call (data + included tiers)
pub fn members_of(page: &Value) -> Vec<Member> {
    let tiers: HashMap<String, String> = page
        .get("included")
        .and_then(Value::as_array)
        .map(|inc| {
            inc.iter()
                .filter(|i| i.get("type").and_then(Value::as_str) == Some("tier"))
                .filter_map(|i| Some((i.get("id")?.as_str()?.to_string(), i.pointer("/attributes/title")?.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    page.get("data")
        .and_then(Value::as_array)
        .map(|data| {
            data.iter()
                .map(|m| {
                    let a = m.get("attributes").cloned().unwrap_or(Value::Null);
                    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let n = |k: &str| a.get(k).and_then(Value::as_i64).unwrap_or(0);
                    let tier = m
                        .pointer("/relationships/currently_entitled_tiers/data/0/id")
                        .and_then(Value::as_str)
                        .and_then(|id| tiers.get(id).cloned())
                        .unwrap_or_default();
                    Member {
                        id: m.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                        name: { let n = s("full_name"); if n.is_empty() { "A patron".into() } else { n } },
                        status: s("patron_status"),
                        cents: n("currently_entitled_amount_cents"),
                        tier,
                        lifetime_cents: n("lifetime_support_cents"),
                        since: s("pledge_relationship_start"),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What changed between two looks: (what, member, amount before)
pub fn changes(before: &HashMap<String, Member>, now: &[Member]) -> Vec<(&'static str, Member, i64)> {
    let mut out = Vec::new();
    for m in now {
        match before.get(&m.id) {
            None if m.active() => out.push(("new", m.clone(), 0)),
            Some(prev) if !prev.active() && m.active() => out.push(("new", m.clone(), 0)),
            Some(prev) if prev.active() && !m.active() => out.push(("end", m.clone(), prev.cents)),
            Some(prev) if m.active() && (prev.cents != m.cents || prev.tier != m.tier) => out.push(("change", m.clone(), prev.cents)),
            _ => {}
        }
    }
    for (id, prev) in before {
        if prev.active() && !now.iter().any(|m| &m.id == id) {
            out.push(("end", prev.clone(), prev.cents));
        }
    }
    out
}

/// The messages for one change (`name`: the trigger's channel)
pub fn messages(what: &str, m: &Member, before_cents: i64, name: &str, test: bool) -> Vec<Value> {
    let (kind, words) = match what {
        "new" => ("pledge", format!("pledge {}", m.tier).trim().to_string()),
        "change" => ("pledge_change", "pledge change".to_string()),
        _ => ("pledge_end", "pledge end".to_string()),
    };
    let trigger = format!("Patreon {name} {words}");
    let amount = if what == "end" { before_cents } else { m.cents } as f64 / 100.0;
    let raw = json!({
        "member_id": m.id, "full_name": m.name, "patron_status": m.status, "tier_title": m.tier,
        "pledge_cents": m.cents, "previous_cents": before_cents, "lifetime_support_cents": m.lifetime_cents,
        "username": m.name, "amount": format!("{:.2}", amount), "pledge_amount": format!("${:.2}", amount), "is_test": test,
    });
    let alert = Alert {
        kind: kind.into(),
        platform: "patreon".into(),
        channel: name.to_string(),
        user: m.name.clone(),
        amount,
        currency: "USD".into(),
        tier: m.tier.clone(),
        test,
        trigger: trigger.clone(),
        raw: raw.clone(),
        ..Default::default()
    };
    vec![
        json!({ "type": "alert", "source": "PatreonAPI", "event": format!("Patreon {}", words.split(' ').take(2).collect::<Vec<_>>().join(" ")), "trigger": trigger, "data": raw, "alert": alert.to_value() }),
        json!({ "type": "moderation", "payload": { "trigger": trigger, "platform": "patreon", "username": m.name, "message": "", "customData": raw } }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, status: &str, cents: i64, tier: &str) -> Member {
        Member { id: id.into(), name: format!("P{id}"), status: status.into(), cents, tier: tier.into(), lifetime_cents: 0, since: String::new() }
    }

    #[test]
    fn reads_a_members_page_with_its_tiers() {
        let page = json!({
            "data": [{ "id": "m1", "type": "member", "attributes": { "full_name": "Ann", "patron_status": "active_patron", "currently_entitled_amount_cents": 500, "lifetime_support_cents": 2500 },
                       "relationships": { "currently_entitled_tiers": { "data": [{ "id": "t1", "type": "tier" }] } } }],
            "included": [{ "id": "t1", "type": "tier", "attributes": { "title": "Gold" } }]
        });
        let m = members_of(&page);
        assert_eq!((m[0].name.as_str(), m[0].tier.as_str(), m[0].cents, m[0].active()), ("Ann", "Gold", 500, true));
    }

    #[test]
    fn new_changed_returning_and_ended_pledges() {
        let before: HashMap<String, Member> = [member("1", "active_patron", 500, "Gold"), member("2", "active_patron", 300, "Silver"), member("3", "former_patron", 0, ""), member("4", "active_patron", 100, "Bronze")]
            .into_iter()
            .map(|m| (m.id.clone(), m))
            .collect();
        let now = vec![
            member("1", "active_patron", 1000, "Platinum"), // upgraded
            member("2", "declined_patron", 300, "Silver"),  // payment failed
            member("3", "active_patron", 500, "Gold"),      // came back
            member("5", "active_patron", 200, "Bronze"),    // new
                                                             // 4 left
        ];
        let mut c: Vec<(&str, String)> = changes(&before, &now).into_iter().map(|(w, m, _)| (w, m.id)).collect();
        c.sort();
        assert_eq!(c, vec![("change", "1".into()), ("end", "2".into()), ("end", "4".into()), ("new", "3".into()), ("new", "5".into())]);
    }

    #[test]
    fn triggers_and_the_shared_alert() {
        let m = member("1", "active_patron", 500, "Gold");
        let out = messages("new", &m, 0, "mycampaign", false);
        assert_eq!(out[0]["trigger"], "Patreon mycampaign pledge Gold");
        assert_eq!((out[0]["alert"]["kind"].as_str(), out[0]["alert"]["amount"].as_f64(), out[0]["alert"]["tier"].as_str()), (Some("pledge"), Some(5.0), Some("Gold")));
        assert_eq!(messages("change", &m, 300, "c", false)[0]["trigger"], "Patreon c pledge change");
        let end = messages("end", &member("2", "former_patron", 0, ""), 300, "c", false);
        assert_eq!((end[0]["trigger"].as_str(), end[0]["alert"]["amount"].as_f64()), (Some("Patreon c pledge end"), Some(3.0)));
    }
}
