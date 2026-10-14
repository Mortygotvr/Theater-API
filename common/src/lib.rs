//! What the Theater API plugins (theater-api-twitch, theater-api-patreon,
//! theater-api-kofi) share. Each is an OBS plugin of its own, installed or
//! not; this is built into each.
//!
//! - being an OBS plugin: `obs_plugin!` makes the exports, and each has a log
//!   in OBS's logs folder and a settings folder (plugin_config\<id>);
//! - its item in the 🎭 Theater menu and its window (obs, window);
//! - the browser login with a Control Panel module's app and proxy (oauth);
//! - sending through the relay, with or without Theater Reader
//!   (theater_shared::relay).

pub mod oauth;
pub mod obs;
pub mod proxies;
pub mod window;

use std::ffi::CStr;
use std::path::PathBuf;
use std::sync::OnceLock;

/// A plugin, as it introduces itself
pub struct Info {
    /// Its folder's name, its DLL's, and its items' in the 🎭 Theater menu.
    /// Never change it.
    pub id: &'static str,
    /// "Theater API · Twitch"
    pub name: &'static str,
    pub name_c: &'static CStr,
    pub description_c: &'static CStr,
    /// Its 🎭 Theater menu item ("🟣 Twitch API…")
    pub menu_title: &'static CStr,
    /// Its window's title
    pub window_title: &'static str,
    /// Its icon (the window's; the menu's is data\icon.png)
    pub icon_png: &'static [u8],
}

static INFO: OnceLock<&'static Info> = OnceLock::new();

pub fn info() -> &'static Info {
    INFO.get().copied().expect("obs_plugin! sets the plugin's info first")
}

/// The plugin's tasks run here.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name(INFO.get().map(|i| i.id).unwrap_or("theater-api"))
            .enable_all()
            .build()
            .expect("Failed to start the plugin's runtime")
    })
}

/// OBS's own folder of the user's (logs, plugin settings)
fn obs_user_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::var("APPDATA").map(PathBuf::from).unwrap_or_default().join("obs-studio")
    } else {
        std::env::var("HOME").map(PathBuf::from).unwrap_or_default().join(".config").join("obs-studio")
    }
}

/// Its settings' folder: OBS's plugin_config\<id>.
pub fn config_dir() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join("theater_api_test_config");
    }
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let id = INFO.get().map(|i| i.id).unwrap_or("theater-api");
        let dir = obs::module_config_dir().unwrap_or_else(|| obs_user_dir().join("plugin_config").join(id));
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

/// A line in OBS's logs folder, <id>.log.
pub fn log(msg: &str) {
    if cfg!(test) {
        return;
    }
    use std::io::Write;
    let id = INFO.get().map(|i| i.id).unwrap_or("theater-api");
    let dir = obs_user_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{}.log", id.replace('-', "_")))) {
        let _ = writeln!(f, "{}: {}", unix_secs(), msg);
    }
}

pub fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A settings file of its folder, as `T` (its default when there's none)
pub fn load_json<T: serde::de::DeserializeOwned + Default>(file: &str) -> T {
    std::fs::read_to_string(config_dir().join(file)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Saves a settings file of its folder (whole, through a temporary file)
pub fn save_json<T: serde::Serialize>(file: &str, value: &T) {
    let path = config_dir().join(file);
    let tmp = path.with_extension("tmp");
    if let Ok(text) = serde_json::to_string_pretty(value) {
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Before the plugin's own load: its info, the panic logger, TLS, the menu.
pub fn init(info: &'static Info) {
    let _ = INFO.set(info);
    install_panic_logger();
    // One TLS for everything (ring)
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Its relay role: a guest, never holding the port (a Reader's, Theater
/// Reader's plugin or app), with what connected apps send (None: not wanted)
pub fn start_relay(on_app: Option<fn(&str)>) -> theater_shared::relay::Role {
    theater_shared::relay::start_guest(info().name, log, on_app)
}

/// Panics in its threads otherwise vanish (OBS has no console on Windows)
fn install_panic_logger() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let thread = std::thread::current();
            log(&format!("[PANIC] thread '{}': {}", thread.name().unwrap_or("<unnamed>"), panic));
            default_hook(panic);
        }));
    });
}

/// Keeps the DLL loaded until OBS exits: its menu item, its procedures and
/// its threads call into it, and may outlive obs_module_unload.
#[cfg(target_os = "windows")]
pub fn pin_module() {
    unsafe extern "system" {
        fn GetModuleHandleExW(flags: u32, module_name: *const u16, module: *mut *mut std::ffi::c_void) -> i32;
    }
    const PIN: u32 = 0x1;
    const FROM_ADDRESS: u32 = 0x4;
    let mut module: *mut std::ffi::c_void = std::ptr::null_mut();
    unsafe {
        GetModuleHandleExW(FROM_ADDRESS | PIN, pin_module as *const u16, &mut module);
    }
}

/// The OBS module's exports. `load` runs after the common setup; `unload`
/// when OBS unloads it.
#[macro_export]
macro_rules! obs_plugin {
    ($info:expr, load: $load:path, unload: $unload:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn obs_module_ver() -> u32 {
            30u32 << 24
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn obs_module_set_pointer(module: *mut std::ffi::c_void) {
            $crate::obs::set_module(module);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn obs_module_name() -> *const std::ffi::c_char {
            $info.name_c.as_ptr()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn obs_module_description() -> *const std::ffi::c_char {
            $info.description_c.as_ptr()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn obs_module_load() -> bool {
            $crate::init(&$info);
            $load();
            $crate::obs::setup_menu();
            $crate::log(&format!("🚀 {} loaded", $info.name));
            true
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn obs_module_unload() {
            $unload();
            $crate::log(&format!("🛑 {} unloaded", $info.name));
        }

        #[cfg(target_os = "windows")]
        #[unsafe(no_mangle)]
        pub extern "system" fn DllMain(_module: *mut std::ffi::c_void, reason: u32, _reserved: *mut std::ffi::c_void) -> i32 {
            if reason == 1 {
                $crate::pin_module();
            }
            1
        }
    };
}
