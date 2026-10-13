//! The "☕ Ko-fi API" window: the verification token and page, the proxy's
//! state, tests, and the last supporters.

use egui::RichText;
use parking_lot::Mutex;
use theater_api_common::window::{bad_color, ok_color, wait_color};

static DRAFT: Mutex<Option<crate::Settings>> = Mutex::new(None);

pub fn ui(ui: &mut egui::Ui) {
    ui.heading("☕ Ko-fi API");
    ui.label("Ko-fi's donations, subscriptions, shop orders and commissions, for Theater Scene, the Control Panel and the alert scripts.");
    ui.add_space(8.0);

    let saved = crate::settings();
    let mut draft = DRAFT.lock().clone().unwrap_or_else(|| saved.clone());
    ui.label(RichText::new("Settings").strong());
    egui::Grid::new("kofi").num_columns(2).show(ui, |ui| {
        ui.label("Verification token");
        ui.add(egui::TextEdit::singleline(&mut draft.verification_token).password(true).desired_width(280.0));
        ui.end_row();
        ui.label("Page (in triggers)");
        ui.add(egui::TextEdit::singleline(&mut draft.page).desired_width(160.0));
        ui.end_row();
        ui.label("Ask every (s)");
        ui.add(egui::DragValue::new(&mut draft.poll_seconds).range(3..=120));
        ui.end_row();
        ui.label("Proxy address");
        ui.add(egui::TextEdit::singleline(&mut draft.relay_url).hint_text("the Control Panel module's (leave empty)").desired_width(280.0));
        ui.end_row();
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Copy the webhook address").clicked() {
            ui.ctx().copy_text(draft.url());
        }
        if ui.button("Open Ko-fi's webhook settings").clicked() {
            theater_api_common::obs::open_url("https://ko-fi.com/manage/webhooks");
        }
    });
    ui.weak("On Ko-fi: paste the webhook address, then copy Ko-fi's verification token here.");
    ui.weak(format!("Triggers: Kofi {p} donation · Kofi {p} subscription · Kofi {p} shop order · Kofi {p} commission", p = draft.page()));
    let changed = format!("{:?}", draft_key(&draft)) != format!("{:?}", draft_key(&saved));
    if changed {
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Save").strong()).clicked() {
                crate::save_settings(draft.clone());
                *DRAFT.lock() = None;
            } else {
                *DRAFT.lock() = Some(draft.clone());
            }
            if ui.button("Cancel").clicked() {
                *DRAFT.lock() = None;
            }
        });
    } else {
        *DRAFT.lock() = None;
    }
    ui.add_space(8.0);

    ui.label(RichText::new("Status").strong());
    let (polling, last_error, events) = crate::status();
    if !polling {
        ui.colored_label(wait_color(), "Off: enter Ko-fi's verification token.");
    } else if !last_error.is_empty() {
        ui.colored_label(bad_color(), last_error);
    } else {
        ui.colored_label(ok_color(), format!("Asking the proxy · {events} event(s) since OBS started"));
    }
    ui.colored_label(
        wait_color(),
        "If the Control Panel's Ko-fi module asks the proxy too, the two take turns getting each donation: an updated Control Panel leaves it to this plugin.",
    );
    ui.add_space(8.0);

    ui.label(RichText::new("Test").strong());
    ui.horizontal_wrapped(|ui| {
        for kind in ["Donation", "Subscription", "Shop Order", "Commission"] {
            if ui.button(kind).clicked() {
                crate::test(kind);
            }
        }
    });
    ui.add_space(8.0);

    let recent = crate::recent();
    if !recent.is_empty() {
        ui.label(RichText::new("Last supporters").strong());
        for r in recent.iter().take(10) {
            ui.label(format!(
                "{} · {} {} · {}",
                r["user"].as_str().unwrap_or(""),
                r["amount"].as_f64().unwrap_or(0.0),
                r["currency"].as_str().unwrap_or(""),
                r["type"].as_str().unwrap_or("")
            ));
        }
    }
}

fn draft_key(s: &crate::Settings) -> (String, String, u64, String) {
    (s.verification_token.clone(), s.page.clone(), s.poll_seconds, s.relay_url.clone())
}
