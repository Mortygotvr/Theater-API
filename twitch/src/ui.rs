//! The "🟣 Twitch API" window: the account (log in with the browser), the
//! live connection, the relay, the rewards, the lists, and tests.

use egui::RichText;
use parking_lot::Mutex;
use theater_api_common::window::{bad_color, ok_color, wait_color};

use crate::{api, live};

static NOTE: Mutex<String> = Mutex::new(String::new());

pub fn set_note(note: &str) {
    *NOTE.lock() = note.to_string();
    theater_api_common::window::repaint();
}

pub fn ui(ui: &mut egui::Ui) {
    ui.heading("🟣 Twitch API");
    ui.label("Your channel's events from Twitch itself, for Theater Scene, the Control Panel and the alert scripts. With or without Theater Reader.");
    ui.add_space(8.0);

    // The account
    ui.label(RichText::new("Account").strong());
    match (api::logged_in(), api::account()) {
        (true, Some(account)) => {
            ui.horizontal(|ui| {
                ui.colored_label(ok_color(), format!("Logged in as {} ({})", account.name, account.login));
                if ui.button("Log out").clicked() {
                    crate::log_out();
                }
            });
            let missing = api::missing_scopes();
            if !missing.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(wait_color(), format!("Not allowed yet: {}.", missing.join(", ")));
                    if ui.button("Log in again to allow them").clicked() {
                        crate::log_in();
                    }
                });
            }
        }
        (true, None) => {
            ui.colored_label(wait_color(), "Checking the login…");
        }
        (false, _) => {
            ui.horizontal_wrapped(|ui| {
                if ui.button(RichText::new("Log in with Twitch").strong()).clicked() {
                    crate::log_in();
                }
                ui.weak("Opens Twitch in your browser: click Authorize there.");
            });
        }
    }
    let note = NOTE.lock().clone();
    if !note.is_empty() {
        ui.label(note);
    }
    ui.add_space(8.0);

    // Live
    ui.label(RichText::new("Live events").strong());
    match live::link() {
        live::Link::Off => {
            ui.weak("Off (not logged in).");
        }
        live::Link::Connecting => {
            ui.colored_label(wait_color(), "Connecting to Twitch…");
        }
        live::Link::Connected => {
            ui.colored_label(ok_color(), format!("Connected · {} event(s) since OBS started", live::events_count()));
        }
        live::Link::Failed(why) => {
            ui.colored_label(bad_color(), why);
        }
    }
    for (topic, why) in live::refused() {
        ui.colored_label(wait_color(), format!("Twitch won't send {topic}: {why}"));
    }
    let relay = theater_shared::relay::status();
    let relay_text = match &relay.role {
        theater_shared::relay::Role::Host if relay.up => format!("Relay: ws://127.0.0.1:{} (hosted here) · {} app(s)", relay.port, relay.apps),
        theater_shared::relay::Role::Host => format!("Relay: starting… {}", relay.problem.clone().unwrap_or_default()),
        role => format!("Relay: through {}", theater_shared::relay::role_name(role)),
    };
    ui.weak(relay_text);
    ui.weak("With Theater Reader too, Reader leaves this channel's Twitch alerts to this plugin, so they don't come twice.");
    ui.add_space(8.0);

    // Tests
    ui.label(RichText::new("Test").strong());
    ui.horizontal_wrapped(|ui| {
        for (label, topic) in [
            ("Redeem", "channel.channel_points_custom_reward_redemption.add"),
            ("Cheer", "channel.cheer"),
            ("Resub", "channel.subscription.message"),
            ("Gift subs", "channel.subscription.gift"),
            ("Raid", "channel.raid"),
            ("Follow", "channel.follow"),
        ] {
            if ui.button(label).clicked() {
                crate::test(topic);
            }
        }
    });
    ui.weak("Sends a test alert (marked as a test) to everything listening.");
    ui.add_space(8.0);

    // Rewards
    let rewards = live::rewards();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Rewards ({})", rewards.len())).strong());
        if api::logged_in() && ui.small_button("Refresh").clicked() {
            theater_api_common::runtime().spawn(live::refresh_rewards());
        }
    });
    let note = live::rewards_note();
    if !note.is_empty() {
        ui.weak(note);
    }
    if !rewards.is_empty() {
        egui::Grid::new("rewards").striped(true).num_columns(3).show(ui, |ui| {
            for r in &rewards {
                ui.label(&r.title);
                ui.label(format!("{} pts", r.cost as i64));
                let mut state = if !r.enabled { "disabled".to_string() } else if r.paused { "paused".into() } else { "on".into() };
                if r.manageable {
                    state.push_str(" · made by Theater");
                }
                ui.weak(state);
                ui.end_row();
            }
        });
        ui.weak("Rewards made on twitch.tv can be read and fire triggers; only rewards made by Theater can be paused or refunded (Twitch's rule).");
    }
    ui.add_space(8.0);

    // Lists
    let subs = live::subscribers();
    let follows = live::followers();
    ui.label(RichText::new("Lists").strong());
    ui.weak(format!("{} subscriber(s), {} follower(s): for scripts and commands (theater_api_twitch_subscribers / _followers).", subs.len(), follows.len()));
}
