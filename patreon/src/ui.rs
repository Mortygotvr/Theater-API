//! The "🧡 Patreon API" window: the account, the campaign, the name in
//! triggers, tests, and the patrons.

use egui::RichText;
use parking_lot::Mutex;
use theater_api_common::window::{bad_color, ok_color, wait_color};

static DRAFT: Mutex<Option<crate::Settings>> = Mutex::new(None);

pub fn ui(ui: &mut egui::Ui) {
    ui.heading("🧡 Patreon API");
    ui.label("Your campaign's new, changed and ended pledges, and your patrons, for Theater Scene, the Control Panel and the alert scripts.");
    ui.add_space(8.0);

    let (campaign, patrons, members, last_error, note) = crate::status();
    ui.label(RichText::new("Account").strong());
    if crate::logged_in() {
        ui.horizontal(|ui| {
            if campaign.is_empty() {
                ui.colored_label(wait_color(), "Logged in · looking at the campaign…");
            } else {
                ui.colored_label(ok_color(), format!("Logged in · {campaign} · {patrons} active patron(s)"));
            }
            if ui.button("Log out").clicked() {
                crate::log_out();
            }
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            if ui.button(RichText::new("Log in with Patreon").strong()).clicked() {
                crate::log_in();
            }
            ui.weak("Opens Patreon in your browser: click Allow there.");
        });
    }
    if !note.is_empty() {
        ui.label(note);
    }
    if !last_error.is_empty() {
        ui.colored_label(bad_color(), last_error);
    }
    ui.add_space(8.0);

    let saved = crate::settings();
    let mut draft = DRAFT.lock().clone().unwrap_or_else(|| saved.clone());
    ui.label(RichText::new("Settings").strong());
    egui::Grid::new("patreon").num_columns(2).show(ui, |ui| {
        ui.label("Name (in triggers)");
        ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(160.0));
        ui.end_row();
        ui.label("Look every (s)");
        ui.add(egui::DragValue::new(&mut draft.poll_seconds).range(30..=900));
        ui.end_row();
    });
    ui.weak(format!("Triggers: Patreon {n} pledge <tier> · Patreon {n} pledge change · Patreon {n} pledge end", n = draft.name()));
    if (draft.name.clone(), draft.poll_seconds) != (saved.name.clone(), saved.poll_seconds) {
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
    ui.weak("Patreon doesn't tell right away: changes show at the next look.");
    ui.add_space(8.0);

    ui.label(RichText::new("Test").strong());
    ui.horizontal(|ui| {
        for (label, what) in [("New pledge", "new"), ("Pledge change", "change"), ("Pledge end", "end")] {
            if ui.button(label).clicked() {
                crate::test(what);
            }
        }
    });
    ui.add_space(8.0);

    if members > 0 {
        ui.label(RichText::new(format!("Patrons ({patrons} active of {members})")).strong());
        egui::Grid::new("patrons").striped(true).num_columns(3).show(ui, |ui| {
            for m in crate::members().iter().filter(|m| m.active()).take(200) {
                ui.label(&m.name);
                ui.label(if m.tier.is_empty() { "—" } else { &m.tier });
                ui.label(format!("${:.2}", m.cents as f64 / 100.0));
                ui.end_row();
            }
        });
    }
}
