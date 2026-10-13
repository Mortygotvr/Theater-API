//! The plugin's window (its 🎭 Theater menu item): one eframe window on a
//! thread of its own, made the first time it's opened. Closing it only hides
//! it (eframe can't start again in the same process). What it shows is the
//! plugin's (`set_ui`).

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use parking_lot::Mutex;

static THREAD_RUNNING: AtomicBool = AtomicBool::new(false);
static VISIBLE: AtomicBool = AtomicBool::new(false);
static SHOW_REQUEST: AtomicBool = AtomicBool::new(false);
static HWND: AtomicIsize = AtomicIsize::new(0);
static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);
static UI: Mutex<Option<fn(&mut egui::Ui)>> = Mutex::new(None);

/// What the window shows (the plugin's, set in its load)
pub fn set_ui(ui: fn(&mut egui::Ui)) {
    *UI.lock() = Some(ui);
}

pub fn is_open() -> bool {
    VISIBLE.load(Ordering::SeqCst)
}

/// The menu item: shows the window, making it the first time.
pub fn open() {
    VISIBLE.store(true, Ordering::SeqCst);
    SHOW_REQUEST.store(true, Ordering::SeqCst);
    if !THREAD_RUNNING.swap(true, Ordering::SeqCst) {
        spawn();
        return;
    }
    show_native();
    repaint();
}

/// Draw it again soon (something it shows changed). Any thread.
pub fn repaint() {
    if let Some(ctx) = CTX.lock().as_ref() {
        ctx.request_repaint();
    }
}

/// Windows doesn't paint a hidden window: shown natively first, its next
/// frame handles SHOW_REQUEST.
fn show_native() {
    #[cfg(target_os = "windows")]
    {
        unsafe extern "system" {
            fn ShowWindowAsync(hwnd: isize, cmd: i32) -> i32;
            fn IsIconic(hwnd: isize) -> i32;
        }
        const SW_SHOW: i32 = 5;
        const SW_RESTORE: i32 = 9;
        let hwnd = HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe { ShowWindowAsync(hwnd, if IsIconic(hwnd) != 0 { SW_RESTORE } else { SW_SHOW }) };
        }
    }
}

fn icon() -> Option<egui::IconData> {
    let rgba = image::load_from_memory(crate::info().icon_png).ok()?.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(egui::IconData { rgba: rgba.into_raw(), width, height })
}

fn spawn() {
    let info = crate::info();
    let _ = std::thread::Builder::new().name(format!("{}-window", info.id)).spawn(move || {
        let mut viewport = egui::ViewportBuilder::default()
            .with_title(info.window_title)
            .with_inner_size([640.0, 600.0])
            .with_min_inner_size([460.0, 340.0]);
        if let Some(icon) = icon() {
            viewport = viewport.with_icon(icon);
        }
        let options = eframe::NativeOptions {
            viewport,
            // Not OBS's main thread
            event_loop_builder: Some(Box::new(|builder| {
                #[cfg(target_os = "linux")]
                {
                    use winit::platform::wayland::EventLoopBuilderExtWayland;
                    use winit::platform::x11::EventLoopBuilderExtX11;
                    if std::env::var("DISPLAY").map(|s| !s.is_empty()).unwrap_or(false) {
                        EventLoopBuilderExtX11::with_x11(builder);
                        EventLoopBuilderExtX11::with_any_thread(builder, true);
                    } else {
                        EventLoopBuilderExtWayland::with_any_thread(builder, true);
                    }
                }
                #[cfg(target_os = "windows")]
                {
                    use winit::platform::windows::EventLoopBuilderExtWindows;
                    EventLoopBuilderExtWindows::with_any_thread(builder, true);
                }
            })),
            ..Default::default()
        };
        let result = eframe::run_native(
            info.id,
            options,
            Box::new(|cc| {
                *CTX.lock() = Some(cc.egui_ctx.clone());
                #[cfg(target_os = "windows")]
                {
                    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    if let Ok(handle) = cc.window_handle() {
                        if let RawWindowHandle::Win32(win32) = handle.as_raw() {
                            HWND.store(win32.hwnd.get(), Ordering::SeqCst);
                        }
                    }
                }
                Ok(Box::new(PluginWindow))
            }),
        );
        if let Err(e) = result {
            crate::log(&format!("❌ The window couldn't run: {:?}", e));
        }
        HWND.store(0, Ordering::SeqCst);
        *CTX.lock() = None;
        VISIBLE.store(false, Ordering::SeqCst);
        THREAD_RUNNING.store(false, Ordering::SeqCst);
    });
}

struct PluginWindow;

impl eframe::App for PluginWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if SHOW_REQUEST.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            VISIBLE.store(false, Ordering::SeqCst);
            return;
        }
        if !is_open() {
            return;
        }
        let ui_fn = *UI.lock();
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if let Some(ui_fn) = ui_fn {
                    ui_fn(ui);
                }
            });
        });
        // Statuses change on their own
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
}

/// A status line's color: green, amber, red
pub fn ok_color() -> egui::Color32 {
    egui::Color32::from_rgb(34, 197, 94)
}
pub fn wait_color() -> egui::Color32 {
    egui::Color32::from_rgb(234, 179, 8)
}
pub fn bad_color() -> egui::Color32 {
    egui::Color32::from_rgb(239, 68, 68)
}
