# Theater API

One OBS plugin per service, each installed or not, on its own:

| Plugin | What it brings | Triggers |
|---|---|---|
| **Theater API · Twitch** (`theater-api-twitch`) | Your channel's events from Twitch itself: redeems with their real titles, cheers, subs, resubs, gifts, raids, follows, hype trains, bans and timeouts, the stream going live. Your rewards, subscribers and followers. | Theater Reader's: `Twitch <channel> redeem <title>`, `… cheer <n> bits`, `… sub`, `… raid`, `… hype train <level>`, `… ban`, `… timeout`; and `… follow`, `… stream start`, `… stream end` |
| **Theater API · Patreon** (`theater-api-patreon`) | New, changed and ended pledges, and your patrons by tier | `Patreon <name> pledge <tier>`, `… pledge change`, `… pledge end` |
| **Theater API · Ko-fi** (`theater-api-kofi`) | Donations, memberships, shop orders, commissions | `Kofi <page> donation`, `… subscription`, `… shop order`, `… commission` |

Each one sends its alerts in Theater's shared alert shape (`theater_shared::alert`), with their triggers. Theater Scene (its triggers and the Rhai alert scripts) and the Control Panel get them like Theater Reader's.

## Working together

- **One Reader at a time, the API plugins always.** The relay's port (`ws://127.0.0.1:41837`) is a Reader's: the Theater Reader plugin's or the Theater Reader app's. The API plugins never take it (`theater_shared::relay::start_guest`), so each works with either Reader, or none, and the other API plugins.
  - **With the Theater Reader plugin,** they send through it (`theater_relay_send`): everything on its port, and once on the `theater_event` OBS signal.
  - **Otherwise** (the Theater Reader app, or no Reader), they send on `theater_event` only. Theater Scene and the Control Panel listen to it inside OBS.
- **No Twitch alert twice.** While Theater API · Twitch is connected to a channel (`theater_api_twitch_channels`), the Theater Reader plugin leaves that channel's Twitch alerts to it. With the Theater Reader app, Theater Scene and the Control Panel leave out the app's Twitch alerts for that channel.
- **The Control Panel's** Ko-fi module stops asking the Ko-fi proxy when Theater API · Ko-fi is installed, because the proxy's queue empties on each read.

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
