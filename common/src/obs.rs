//! The plugin in OBS: its module's folders, and its item in the shared
//! 🎭 Theater menu (theater-control-panel/docs/shared-menu.md), else in OBS's
//! Tools menu. Procedures and signals: theater_shared::obs_link.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, Ordering};

type MenuCb = unsafe extern "C" fn(data: *mut c_void);

/// OBS's handle of this module (obs_module_set_pointer)
static MODULE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn set_module(module: *mut c_void) {
    MODULE.store(module, Ordering::SeqCst);
}

#[cfg(target_os = "windows")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, file: *mut u16, size: u32) -> u32;
}

#[cfg(not(target_os = "windows"))]
unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

#[cfg(target_os = "windows")]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const LIBOBS: &str = if cfg!(target_os = "windows") { "obs.dll" } else { "" };
const FRONTEND: &str = if cfg!(target_os = "windows") { "obs-frontend-api.dll" } else { "libobs-frontend-api.so.0" };

fn symbol(lib: &str, name: &CStr) -> *mut c_void {
    #[cfg(target_os = "windows")]
    {
        let module = unsafe { GetModuleHandleW(wide(lib).as_ptr()) };
        if module.is_null() {
            return std::ptr::null_mut();
        }
        unsafe { GetProcAddress(module, name.as_ptr()) }
    }
    #[cfg(not(target_os = "windows"))]
    {
        const RTLD_NOW: i32 = 2;
        let handle = if lib.is_empty() {
            std::ptr::null_mut()
        } else {
            let Ok(lib) = CString::new(lib) else { return std::ptr::null_mut() };
            unsafe { dlopen(lib.as_ptr(), RTLD_NOW) }
        };
        unsafe { dlsym(handle, name.as_ptr()) }
    }
}

fn function<T>(lib: &str, name: &CStr) -> Option<T> {
    let p = symbol(lib, name);
    // SAFETY: T is the function pointer type of the export `name`.
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy(&p) })
}

/// A string libobs allocated, as a path, freed
fn take_obs_string(p: *mut c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    let path = PathBuf::from(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned());
    if let Some(bfree) = function::<unsafe extern "C" fn(*mut c_void)>(LIBOBS, c"bfree") {
        unsafe { bfree(p as *mut c_void) };
    }
    Some(path)
}

/// OBS's config folder for this plugin (obs_module_config_path's)
pub fn module_config_dir() -> Option<PathBuf> {
    let module = MODULE.load(Ordering::SeqCst);
    if module.is_null() {
        return None;
    }
    let get: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char = function(LIBOBS, c"obs_module_get_config_path")?;
    take_obs_string(unsafe { get(module, c"".as_ptr()) })
}

/// A file of the plugin's data folder (obs_module_file's)
pub fn module_file(file: &CStr) -> Option<PathBuf> {
    let module = MODULE.load(Ordering::SeqCst);
    if module.is_null() {
        return None;
    }
    let find: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char = function(LIBOBS, c"obs_find_module_file")?;
    take_obs_string(unsafe { find(module, file.as_ptr()) })
}

/// Opens a web page in the default browser (where the person is logged in).
pub fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    {
        unsafe extern "system" {
            fn ShellExecuteW(hwnd: *mut c_void, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
        }
        const SW_SHOWNORMAL: i32 = 1;
        unsafe {
            ShellExecuteW(std::ptr::null_mut(), wide("open").as_ptr(), wide(url).as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

// ---------------------------------------------------------------------------
// The 🎭 Theater menu

const OBS_TASK_UI: u32 = 0;

unsafe extern "C" fn open_window_cb(_data: *mut c_void) {
    crate::window::open();
}

/// Adds its item to the 🎭 Theater menu once OBS's main window is up.
pub fn setup_menu() {
    let _ = std::thread::Builder::new().name(format!("{}-menu", crate::info().id)).spawn(|| {
        if let Some(get_main_window) = function::<unsafe extern "C" fn() -> *mut c_void>(FRONTEND, c"obs_frontend_get_main_window") {
            for _ in 0..150 {
                if !unsafe { get_main_window() }.is_null() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        type QueueTask = unsafe extern "C" fn(u32, unsafe extern "C" fn(*mut c_void), *mut c_void, bool);
        match function::<QueueTask>(LIBOBS, c"obs_queue_task") {
            Some(queue_task) => unsafe { queue_task(OBS_TASK_UI, add_menu_item_task, std::ptr::null_mut(), false) },
            None => unsafe { add_menu_item_task(std::ptr::null_mut()) },
        }
    });
}

/// On OBS's UI thread: the 🎭 Theater menu, else OBS's Tools menu.
unsafe extern "C" fn add_menu_item_task(_data: *mut c_void) {
    let info = crate::info();
    if add_to_theater_menu() {
        crate::log("🎭 Added its item to the Theater menu");
        return;
    }
    type AddToolsItem = unsafe extern "C" fn(*const c_char, MenuCb, *mut c_void);
    match function::<AddToolsItem>(FRONTEND, c"obs_frontend_add_tools_menu_item") {
        Some(add) => {
            unsafe { add(info.menu_title.as_ptr(), open_window_cb, std::ptr::null_mut()) };
            crate::log("🔧 No Qt shim: added its item to OBS's Tools menu");
        }
        None => crate::log("⚠️ Couldn't add a menu item: no obs-frontend-api"),
    }
}

/// Through the Qt shim shipped next to this DLL (trust_qt.dll, Theater
/// Scene's own: it takes any plugin's id and icon).
fn add_to_theater_menu() -> bool {
    let Some(shim) = load_shim() else { return false };
    type StartGroup = unsafe extern "C" fn(*const c_char) -> bool;
    type AddItem = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, MenuCb, *mut c_void) -> bool;
    let (Some(start_group), Some(add_item)) = (
        shim_function::<StartGroup>(shim, c"trust_ui_menu_start_group"),
        shim_function::<AddItem>(shim, c"trust_ui_menu_add_item"),
    ) else {
        crate::log("⚠️ The Qt shim is too old for the Theater menu (no trust_ui_menu_add_item)");
        return false;
    };
    let info = crate::info();
    let id = CString::new(info.id).unwrap_or_default();
    let icon = module_file(c"icon.png").map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let icon = CString::new(icon).unwrap_or_default();
    unsafe { start_group(id.as_ptr()) && add_item(id.as_ptr(), icon.as_ptr(), info.menu_title.as_ptr(), open_window_cb, std::ptr::null_mut()) }
}

/// The folder this DLL is in
#[cfg(target_os = "windows")]
fn own_dir() -> Option<PathBuf> {
    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;
    let mut module: *mut c_void = std::ptr::null_mut();
    if unsafe { GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, own_dir as *const u16, &mut module) } == 0 {
        return None;
    }
    let mut buf = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (len > 0 && len < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len])))?.parent().map(PathBuf::from)
}

#[cfg(target_os = "windows")]
fn load_shim() -> Option<*mut c_void> {
    let path = own_dir()?.join("trust_qt.dll");
    if !path.is_file() {
        crate::log(&format!("No Qt shim at {}", path.display()));
        return None;
    }
    let handle = unsafe { LoadLibraryW(wide(&path.to_string_lossy()).as_ptr()) };
    (!handle.is_null()).then_some(handle)
}

#[cfg(not(target_os = "windows"))]
fn load_shim() -> Option<*mut c_void> {
    None
}

fn shim_function<T>(shim: *mut c_void, name: &CStr) -> Option<T> {
    #[cfg(target_os = "windows")]
    let p = unsafe { GetProcAddress(shim, name.as_ptr()) };
    #[cfg(not(target_os = "windows"))]
    let p = unsafe { dlsym(shim, name.as_ptr()) };
    // SAFETY: T is the function pointer type of the shim's export `name`.
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy(&p) })
}
