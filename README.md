# Theater API

One OBS plugin per service, each installed or not, on its own:

| Plugin | What it brings | Triggers |
|---|---|---|
| **Theater API · Twitch** (`theater-api-twitch`) | Your channel's events from Twitch itself: redeems with their real titles, cheers, subs, resubs, gifts, raids, follows, hype trains, bans and timeouts, the stream going live. Your rewards, subscribers and followers. | Theater Reader's: `Twitch <channel> redeem <title>`, `… cheer <n> bits`, `… sub`, `… raid`, `… hype train <level>`, `… ban`, `… timeout`; and `… follow`, `… stream start`, `… stream end` |
| **Theater API · Patreon** (`theater-api-patreon`) | New, changed and ended pledges, and your patrons by tier | `Patreon <name> pledge <tier>`, `… pledge change`, `… pledge end` |
| **Theater API · Ko-fi** (`theater-api-kofi`) | Donations, memberships, shop orders, commissions | `Kofi <page> donation`, `… subscription`, `… shop order`, `… commission` |

Each one sends its alerts through the relay (`ws://127.0.0.1:41837`) in Theater's shared alert shape (`theater_shared::alert`), plus the trigger feed. Theater Scene, the Control Panel and the Rhai alert scripts get them like Theater Reader's.

## Working together

- **The relay** belongs to whichever Theater plugin starts first (`theater_shared::relay`). The others send through it. Each plugin therefore works with or without Theater Reader and the other plugins.
- **With Theater Reader too,** Reader leaves a channel's Twitch alerts to Theater API · Twitch while it's connected (`theater_api_twitch_channels`), so no alert comes twice.
- **The Control Panel** reads everything once, from the `theater_event` OBS signal. Its Ko-fi module stops asking the Ko-fi proxy when Theater API · Ko-fi is installed, because the proxy's queue empties on each read.

## Logins

- **The Control Panel modules' apps and Google Apps Script proxies are reused** (`common/src/proxies.rs`). The apps' secrets stay in the proxies.
- **Twitch and Patreon:** the default browser opens the authorize page; Twitch or Patreon sends the code to `http://localhost:52246/`; the proxy trades it for tokens. Tokens are kept in `plugin_config\theater-api-<name>\token.json` and refreshed through the proxy.
- **Ko-fi:** Ko-fi's webhook posts to the proxy, and the plugin asks the proxy for what came (`?action=poll&token=<verification token>`).

## OBS procedures

Each plugin's procedures are listed at the top of its `lib.rs`: state, lists (rewards, subscribers, followers, patrons, recent supporters), and sending a Twitch chat message.

## Build

```powershell
cargo build --release
cargo test --workspace
.\deploy.ps1            # into C:\ProgramData\obs-studio\plugins (OBS closed)
.\package.ps1           # dist\theater-api-<name>-<version>-windows-x64.zip
```

The plugins need Theater-Shared next to this folder (`..\Theater-Shared`), and Theater Scene's Qt shim (`trust_qt.dll`) for their 🎭 Theater menu items.

## License

GPL-2.0-or-later.
