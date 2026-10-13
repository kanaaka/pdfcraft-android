//! PdfCraft on Android (arm64-v8a): the real engine and egui shell, running on winit's
//! NativeActivity backend.
//!
//! # What this crate is
//!
//! A shell around the *unmodified* PdfCraft engine and UI. It supplies what `apps/pdfcraft`
//! supplies on the desktop: the preferences file, the recovery folder, the window options, the
//! log, the panic hook — and, instead of a native file dialog, a picker drawn with egui.
//!
//! # The one upstream change a port of this app needs
//!
//! PdfCraft is not built like its siblings. `gridcraft-ui-egui`, `wordcraft-ui-egui` and
//! `deckcraft-ui-egui` reach the platform through a `Services` struct of `Option<Box<dyn Fn…>>`
//! that a port fills in; `pdfcraft-ui-egui` calls `rfd::AsyncFileDialog` at 42 call sites instead.
//! `rfd` has no Android backend (`backend.rs` compiles no module for a target that is neither
//! Linux/BSD, macOS, Windows nor wasm32), and Cargo refuses to swap the crate for a local one per
//! target ("different source paths depending on the build target").
//!
//! So `pdfcraft-ui-egui` resolves the name itself, in `crates/ui-egui/src/rfd.rs`: the call sites
//! say `crate::rfd::…`, which is the real `rfd` on the desktop and wasm and a host-backed
//! stand-in on Android. That module asks *this crate* to show the panel — [`install_picker`]
//! hands it a `Fn(Request) -> Receiver<Vec<PathBuf>>`, and [`file_picker`] answers on that
//! receiver. The desktop build is untouched: `pub use ::rfd::*` is the same crate it always used.
//! `create_ui.rs` and `zoom_snap.rs` also had their `arboard` use target-gated away, the same way
//! their existing wasm arms already were.
//!
//! # Feature notes
//!
//! - `android-native-activity` **must** be on, and `accesskit` **must** be off — eframe
//!   `compile_error!`s that pair, because egui's accesskit integration on Android goes through
//!   GameActivity while this packages NativeActivity. This crate's manifest enables the first;
//!   `pdfcraft-ui-egui`'s declares eframe a second time under `cfg(not(target_os = "android"))`
//!   for the second, which is the only way to drop a feature the graph would otherwise union in.
//! - `NativeOptions::android_app` must be `Some(..)`: eframe hands it to
//!   `EventLoopBuilderExtAndroid::with_android_app` and returns an error without it.
//!
//! # What Android cannot do yet
//!
//! Honest gaps, not oversights:
//!
//! - **File dialogs are drawn by this crate** ([`file_picker`]). Open, Save As, Export, Combine,
//!   Insert, OCR, redaction word lists, folder pins and the create-from-images pickers all go
//!   through `crate::rfd`, so all of them reach it. It browses the app's own folder with no
//!   permission at all and offers a shared-storage root only when it can actually read it.
//!   Android's *own* picker is still future work: the Storage Access Framework needs
//!   `startActivityForResult`, which `android-activity` 0.6 does not expose.
//! - **No system clipboard.** `arboard` has no Android backend. Create ▸ Clipboard says the same
//!   thing it says on the web ("The clipboard has no image or text"), and the snapshot zoom
//!   still records its `last_snapshot`; only the system-wide clipboard is out of reach.
//! - **No printing.** `printer_source` is left unset, so File ▸ Print has no printer list to show.
//! - **No update check or link opening.** `update_source` and the external links need an
//!   `Intent`/`open` through JNI and are left unset.
//! - **No stylus pressure or tilt.** A pen degrades to ordinary touch.
//! - **No quit prompt.** PdfCraft has no File ▸ Exit; quitting is the window closing (macOS Dock,
//!   Windows/Linux close button) or the control channel. Android closes an app by finishing its
//!   activity, and `ViewportCommand::Close` is a no-op in winit's Android backend, so the shell
//!   does not intercept anything — instead the preferences are written every [`PREFS_SAVE_SECS`]
//!   seconds, because a swipe from Recents runs no `on_exit`, and unsaved documents are covered
//!   by PdfCraft's own recovery store, which this crate points at the app's data folder.
//!
//! The PDF engine, the renderer, the editing commands and the file formats are platform-neutral,
//! and the interface fonts are embedded, so text works with no system font folder.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use pdfcraft_ui_egui::PdfCraftApp;

mod file_picker;
mod modifiers;
mod prefs_watch;
mod touch_scroll;

/// The app's private data directory, captured in `android_main` before the first frame:
/// `<internal storage>/Android/data/ai.storyteller.pdfcraft/files`.
///
/// `internal_data_path()` needs no runtime permission — it is the app's own sandbox — which is why
/// the preferences, the AutoRecover folder and the log land here rather than in shared storage.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// How often the preferences are looked at; a change is written at the next look. Short enough
/// that nothing a user would miss is stale, long enough to cost nothing while the app is idle.
const PREFS_EVERY: Duration = Duration::from_secs(2);

/// The same file the desktop keeps in its eframe storage under the `pdfcraft` key.
fn prefs_file() -> Option<PathBuf> {
    DATA_DIR.get().map(|d| d.join("ui.json"))
}

/// Mirrors the desktop's `cc.storage.get_string("pdfcraft")` → `app.restore`.
fn load_prefs(app: &mut PdfCraftApp) {
    if let Some(p) = prefs_file()
        && let Ok(json) = std::fs::read_to_string(&p)
    {
        app.restore(&json);
    }
}

/// What goes to disk: exactly what the desktop hands eframe's storage
/// (`storage.set_string("pdfcraft", app.persist())`), in a file this shell owns.
fn prefs_bytes(app: &PdfCraftApp) -> Vec<u8> {
    app.persist().into_bytes()
}

/// Hand `pdfcraft_ui_egui::rfd` the picker this crate draws.
///
/// The shim calls this for every `pick_file`/`pick_files`/`pick_folder`/`save_file`, on the UI
/// thread, and waits for the answer on the receiver it returns — from a worker thread, exactly as
/// `pickers.rs` waited on `rfd`'s own future. An empty answer is a cancellation.
#[cfg(target_os = "android")]
fn install_picker() {
    use pdfcraft_ui_egui::rfd::{Kind, Request};

    pdfcraft_ui_egui::rfd::install_picker(Box::new(|request: Request| {
        let (kind, title) = match request.kind {
            Kind::File => (file_picker::Kind::Open, "Open \u{2014} Android"),
            Kind::Files => (file_picker::Kind::OpenMany, "Open \u{2014} Android"),
            Kind::Folder => (file_picker::Kind::Folder, "Choose Folder \u{2014} Android"),
            Kind::Save => (file_picker::Kind::Save, "Save As \u{2014} Android"),
        };
        // The filters are flattened by the shim; an empty list lists every file, which is what a
        // dialog that named no filter asks for.
        let extensions = (!request.extensions.is_empty()).then_some(request.extensions);
        file_picker::request(kind, title, extensions, request.suggested_name)
    }));
}

/// Nothing to install where `rfd` is the real thing; the desktop build never comes through here.
#[cfg(not(target_os = "android"))]
fn install_picker() {}

/// A minimal file logger, so a sideloaded APK leaves a readable trace.
///
/// `logcat` is only reachable over adb; a file in the app's own data directory can be pulled with
/// any file manager, which is what a sideloaded build needs.
struct FileLogger {
    sink: Mutex<Option<std::fs::File>>,
}

impl log::Log for FileLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if let Ok(mut guard) = self.sink.lock() {
            if let Some(f) = guard.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "[{}] {} {}", record.level(), record.target(), record.args());
            }
        }
    }
    fn flush(&self) {}
}

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

fn init_logging(dir: &Path) {
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("pdfcraft.log")) else {
        return;
    };
    let logger = LOGGER.get_or_init(|| FileLogger { sink: Mutex::new(None) });
    if let Ok(mut guard) = logger.sink.lock() {
        *guard = Some(file);
    }
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// The Android entry point.
///
/// `android-activity` spawns a dedicated thread for this and calls it with the activity handle.
/// `#[unsafe(no_mangle)]` with the Rust ABI is the required shape (edition 2024 requires the
/// `unsafe(..)` wrapper around `no_mangle`), and it is the symbol the injected NativeActivity
/// looks for.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    // Count this window while its loop runs (see `prefs_watch`).
    let _window = prefs_watch::counted_window();
    let data_dir = app.internal_data_path().unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = DATA_DIR.set(data_dir.clone());
    init_logging(&data_dir);
    log::info!("PdfCraft {} starting on Android; data dir {}", env!("CARGO_PKG_VERSION"), data_dir.display());
    // The app's external files directory: always readable and writable with no permission, so the
    // picker offers it as a root and Save suggests it first (see `file_picker`).
    if let Some(ext) = app.external_data_path() {
        let _ = std::fs::create_dir_all(&ext);
        let _ = file_picker::EXT_DIR.set(ext);
    }
    // Before the first frame: every dialog the UI asks for goes through this.
    install_picker();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("PdfCraft").with_app_id("ai.storyteller.pdfcraft"),
        // Required on Android: eframe hands this to
        // `EventLoopBuilderExtAndroid::with_android_app` and errors without it.
        android_app: Some(app),
        ..Default::default()
    };

    let result = eframe::run_native(
        "PdfCraft",
        options,
        Box::new(|cc| {
            let mut app = PdfCraftApp::new();
            // First start: the region's unit, as the desktop does on a settings folder it has not
            // seen before.
            app.units = pdfcraft_ui_egui::units::Unit::system();
            load_prefs(&mut app);
            // PdfCraft's own AutoRecover: unsaved documents are written here and offered back
            // after the app is killed, which is the normal way an Android app goes away.
            if let Some(dir) = DATA_DIR.get().map(|d| d.join("Recovery")) {
                let _ = std::fs::create_dir_all(&dir);
                app.enable_recovery(pdfcraft_ui_egui::RecoveryStore::new(dir));
            }
            // The macOS switch that leaves room for the traffic lights inside the app's own title
            // bar; on Android nothing draws them, so it stays off (as on Windows and Linux).
            app.integrated_titlebar = false;
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                log::info!("wgpu adapter: {info:?}; max_texture_dimension_2d {:?}", rs.device.limits().max_texture_dimension_2d);
            } else {
                log::warn!("no wgpu render state");
            }
            Ok(Box::new(Shell {
                inner: app,
                modifiers: modifiers::Tracked::default(),
                touch: touch_scroll::TouchScroll::default(),
                prefs: prefs_watch::Writer::new(prefs_file(), PREFS_EVERY),
            }))
        }),
    );

    match result {
        Ok(()) => log::info!("PdfCraft exited normally"),
        Err(e) => log::error!("PdfCraft stopped: {e}"),
    }
}

/// Wraps the real app so the platform can act on the frame around it: the picker, and the
/// preferences written as it runs.
///
/// PdfCraft is the one app here whose `PdfCraftApp` *is* the `eframe::App` — its siblings keep a
/// separate shell and forward to inherent methods. So this forwards through the trait
/// (`eframe::App::ui(&mut self.inner, ..)`) rather than to an inherent method of the same name.
struct Shell {
    inner: PdfCraftApp,
    /// The preferences, written as the app runs rather than only on the way out: Android never
    /// gives that ending, so a swipe from Recents would otherwise take every preference with it
    /// (see [`prefs_watch`]).
    prefs: prefs_watch::Writer,
    /// Modifier keys, tracked here because the platform never reports them: winit's Android
    /// backend sends no `ModifiersChanged` at all, which is why Shift+click and every Ctrl
    /// shortcut are dead without this (see `modifiers`).
    modifiers: modifiers::Tracked,
    /// Two-finger scrolling, translated from touches into wheel events (see [`touch_scroll`]).
    touch: touch_scroll::TouchScroll,
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        eframe::App::logic(&mut self.inner, ctx, frame);
        if self.prefs.due() {
            let bytes = prefs_bytes(&self.inner);
            self.prefs.store(&bytes);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Before the app, so the frame it sees already has the modifiers a desktop backend would
        // have reported (see `modifiers`).
        let modifiers = self.modifiers.apply(raw);
        // Then the touches: winit's Android backend has no mouse and so sends no wheel events at
        // all, which is why nothing scrolled. A two-finger drag becomes one here (see
        // `touch_scroll`).
        self.touch.apply(raw, modifiers);
        eframe::App::raw_input_hook(&mut self.inner, ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.inner, ui, frame);
        // After the app, so the picker floats above the pages and its docked panels.
        file_picker::draw(ui.ctx());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::App::save(&mut self.inner, storage);
    }

    fn on_exit(&mut self) {
        let bytes = prefs_bytes(&self.inner);
        self.prefs.store_now(&bytes);
    }
}
