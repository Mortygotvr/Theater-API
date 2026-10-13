//! The Control Panel modules' Google Apps Script proxies (twitch_api.tcp,
//! patreon.tcp, kofi.tcp), reused as they are: they hold the apps' client
//! secrets, which never ship in a plugin.

/// Twitch: OAuth code exchange and refresh
pub const TWITCH: &str = "https://script.google.com/macros/s/AKfycbzQx7bjEcYCwviBjFi5USE3q0-QfHNrcqrj0mJVgR38xVb-ogvL8BIIdthumOw7NaRc8A/exec";
/// Patreon: OAuth code exchange and refresh
pub const PATREON: &str = "https://script.google.com/macros/s/AKfycbwo87hHrjSQhkoqMZDZKKQiCVfZAutRrQ2t0AiAQmRMMxCfap7jZ7LJHBrLaQpp0vQZ/exec";
/// Ko-fi: the webhook's queue (?action=poll&token=…)
pub const KOFI: &str = "https://script.google.com/macros/s/AKfycbzRJbtcDnvFcCx-uwsbCdEp1XqHkfrcn8sn-fwB3CtHV22jOGl6QHGyaJE5TY0SNhDP/exec";
