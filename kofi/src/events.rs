//! Ko-fi's webhook events (as the kofi module's proxy queues them) as
//! Theater's messages: the alert, with the shared alert, and the trigger feed.
//! Triggers: "Kofi <page> donation", "Kofi <page> subscription",
//! "Kofi <page> shop order", "Kofi <page> commission".

use serde_json::{Value, json};
use theater_shared::alert::Alert;

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// "5.00", 5, "$5" → 5.0
fn amount(v: &Value) -> f64 {
    match v.get("amount") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// The event inside a queued item (Ko-fi posts it as a form field "data",
/// a JSON text)
pub fn unwrap(item: &Value) -> Value {
    match item.get("data") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| item.clone()),
        Some(v @ Value::Object(_)) if item.get("type").is_none() => v.clone(),
        _ => item.clone(),
    }
}

/// Ko-fi's own test webhook (its dashboard's "Send test"), as the module tells it
pub fn is_kofi_test(e: &Value) -> bool {
    let low = |k: &str| text(e, k).to_lowercase();
    let ids = low("message_id") + " " + &low("kofi_transaction_id");
    ids.contains("12345678-1234-1234-1234-123456789abc")
        || ids.contains("00000000-1111-2222-3333-444444444444")
        || low("from_name").contains("jo example")
        || low("message").contains("good luck with the integration")
        || low("email").contains("jo.example@example.com")
        || low("verification_token") == "your_token"
}

/// Its id, for not showing it twice
pub fn id(e: &Value) -> String {
    let m = text(e, "message_id");
    if m.is_empty() { text(e, "kofi_transaction_id") } else { m }
}

/// The messages for one Ko-fi event (`page`: the trigger's channel)
pub fn messages(e: &Value, page: &str, test: bool) -> Vec<Value> {
    let kind_text = {
        let t = text(e, "type");
        if t.is_empty() { "Donation".to_string() } else { t }
    };
    let (kind, words) = match kind_text.to_lowercase().as_str() {
        "subscription" => ("pledge", "subscription"),
        "shop order" => ("shop_order", "shop order"),
        "commission" => ("commission", "commission"),
        _ => ("donation", "donation"),
    };
    let public = e.get("is_public").and_then(Value::as_bool).unwrap_or(true);
    let name = text(e, "from_name");
    let user = if name.is_empty() || !public { "Someone".to_string() } else { name };
    let items: Vec<String> = e
        .get("shop_items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|i| {
                    let qty = i.get("quantity").and_then(Value::as_u64).unwrap_or(1);
                    let n = text(i, "variation_name");
                    format!("{qty}x {}", if n.is_empty() { text(i, "direct_link_code") } else { n })
                })
                .collect()
        })
        .unwrap_or_default();
    let trigger = format!("Kofi {page} {words}");
    let alert = Alert {
        kind: kind.into(),
        platform: "kofi".into(),
        channel: page.to_string(),
        user: user.clone(),
        anonymous: !public,
        message: if public { text(e, "message") } else { String::new() },
        amount: amount(e),
        currency: text(e, "currency"),
        tier: text(e, "tier_name"),
        title: items.join(", "),
        test,
        trigger: trigger.clone(),
        raw: e.clone(),
        ..Default::default()
    };
    let mut data = e.as_object().cloned().unwrap_or_default();
    // Reader-style keys, for what reads them ({username}, {message}, {amount})
    data.insert("username".into(), Value::from(user.clone()));
    data.insert("message".into(), Value::from(alert.message.clone()));
    if test {
        data.insert("is_test".into(), Value::from(true));
    }
    vec![
        json!({ "type": "alert", "source": "KofiAPI", "event": format!("Kofi {words}"), "trigger": trigger, "msg_id": id(e), "data": data, "alert": alert.to_value() }),
        json!({ "type": "moderation", "payload": { "trigger": trigger, "msg_id": id(e), "platform": "kofi", "username": user, "message": alert.message, "customData": data } }),
    ]
}

/// A test event (the window's Test buttons)
pub fn sample(kind: &str) -> Value {
    json!({
        "type": kind, "from_name": "TestSupporter", "message": "Love the streams!", "amount": "5.00", "currency": "USD",
        "is_public": true, "tier_name": if kind == "Subscription" { "Bronze" } else { "" },
        "shop_items": if kind == "Shop Order" { json!([{ "variation_name": "Sticker pack", "quantity": 2 }]) } else { Value::Null },
        "message_id": format!("theater-test-{}", theater_api_common::unix_secs()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_donation_a_subscription_a_shop_order_and_a_private_one() {
        let d = messages(&json!({"type":"Donation","from_name":"Kind","message":"hi","amount":"5.00","currency":"USD","is_public":true,"message_id":"m1"}), "mypage", false);
        assert_eq!(d[0]["trigger"], "Kofi mypage donation");
        let a = &d[0]["alert"];
        assert_eq!((a["kind"].as_str(), a["user"].as_str(), a["amount"].as_f64(), a["currency"].as_str()), (Some("donation"), Some("Kind"), Some(5.0), Some("USD")));
        assert_eq!(d[0]["msg_id"], "m1");
        assert_eq!(d[1]["payload"]["trigger"], "Kofi mypage donation");
        let s = messages(&json!({"type":"Subscription","from_name":"Sub","amount":"3","tier_name":"Bronze"}), "p", false);
        assert_eq!((s[0]["trigger"].as_str(), s[0]["alert"]["kind"].as_str(), s[0]["alert"]["tier"].as_str()), (Some("Kofi p subscription"), Some("pledge"), Some("Bronze")));
        let o = messages(&json!({"type":"Shop Order","from_name":"Buyer","amount":"10","shop_items":[{"variation_name":"Sticker","quantity":2}]}), "p", false);
        assert_eq!((o[0]["trigger"].as_str(), o[0]["alert"]["title"].as_str()), (Some("Kofi p shop order"), Some("2x Sticker")));
        let private = messages(&json!({"type":"Donation","from_name":"Secret","message":"shh","amount":"1","is_public":false}), "p", false);
        assert_eq!((private[0]["alert"]["user"].as_str(), private[0]["alert"]["message"].as_str()), (Some("Someone"), Some("")));
    }

    #[test]
    fn a_queued_form_post_and_kofis_own_test() {
        let item = json!({"data": "{\"type\":\"Donation\",\"from_name\":\"Jo Example\",\"amount\":\"3.00\",\"message\":\"Good luck with the integration!\"}"});
        let e = unwrap(&item);
        assert_eq!(e["from_name"], "Jo Example");
        assert!(is_kofi_test(&e));
        assert!(!is_kofi_test(&json!({"from_name":"Real Person","message":"hi"})));
    }
}
