# Port notes — PdfCraft on Android

Upstream: [`storytold/pdfcraft`](https://github.com/storytold/pdfcraft) @
`3d2bdfe71ada10f6bc39afc680dce61bbf080cbc` (v0.6.0)
Published APK: `pdfcraft-0.6.0-arm64-v8a-v3.apk`

The port is `apps/pdfcraft-android/` (`src/lib.rs` 289 lines, `src/file_picker.rs` 403,
`src/modifiers.rs` 92, `src/prefs_watch.rs` 134) plus **22 upstream files** — the largest patch in this
set, and almost all of it is one problem: PdfCraft calls `rfd` directly instead of going through a
`Services` seam, and `rfd` does not exist on Android.

## 1. The shell crate

`pdfcraft_ui_egui::Services` is a struct of `Option<Box<dyn Fn…>>` fields deriving `Default`; the
desktop binary and the browser shell fill it their own way, and this is the third. Load-bearing in
every port of this set:

- **`[lib] name = "pdfcraft"` + `crate-type = ["cdylib"]`** → `lib/arm64-v8a/libpdfcraft.so`, and the
  injected `android.app.lib_name` meta-data is the artifact name with `-` → `_`. A mismatch installs
  and then dies on launch.
- **`options.android_app = Some(app)`** — `eframe::run_native` errors without it.
- **A file logger in the app's own data directory**, because `logcat` needs adb and a sideloaded build
  has none.
- **Preferences under `internal_data_path()`**, written while the app runs (§5).

## 2. `rfd` has no Android backend — why the name is resolved *inside* the crate

`rfd`'s `backend.rs` compiles no module for a target that is neither Linux/BSD, macOS, Windows nor
wasm32: the `FilePickerDialogImpl` family is declared and nothing implements it, so the build stops at
the first call with `the trait bound FileDialog: FilePickerDialogImpl is not satisfied`. Android's own
picker is not a substitute, because the Storage Access Framework needs `startActivityForResult`, which
`android-activity` 0.6 does not expose.

The obvious fix — swap in a stand-in crate called `rfd` under a target table — is refused by Cargo:

```
Dependency 'rfd' has different source paths depending on the build target.
Each dependency must have a single canonical source path irrespective of build target.
```

So the name is resolved one hop inside the crate instead. `crates/ui-egui/src/rfd.rs` (new) is the real
`rfd` wherever it exists — `pub use ::rfd::*`, the same crate, same types, same behaviour — and on
Android it is `rfd_android.rs` (new), a drop-in for the small part this crate uses:
`AsyncFileDialog` with its filters and its four `pick_*`/`save_file` futures, and `FileHandle`.

All 42 call sites across 20 files now say `crate::rfd::…`. That is the whole of the patch's bulk, and
it is mechanical: the calls are unchanged, only what answers them is.

## 3. How a picker answer gets back

`pickers.rs` creates the dialog future on the UI thread (macOS shows the panel there) and blocks a
worker thread on it. The port keeps that shape exactly:

1. `pick_file()` / `save_file()` / … **register the request with the host immediately** — `ask` is
   called during `ui`, the only moment with an egui frame to draw in — and return a future waiting on
   a channel.
2. The shell draws the picker with egui and answers on that channel when the user chooses or cancels
   (an empty answer already means cancelled to `pickers.rs`).
3. The worker wakes, queues the pick, and wakes the window; the app uses it on a later frame.

Blocking that worker is deliberate: it is a dedicated thread, which is what `rfd`'s own Linux backend
does, and polling the channel from `poll` would need a waker the picker has no way to reach. When no
picker is installed — a test, or a host that forgot — the request is "cancelled" immediately with an
error in the log, because waiting for a dialog that can never be drawn looks exactly like a hang.

The picker drawing rules are the ones the sibling ports share: sized from `ctx.content_rect()`,
enlarged padding for fingers, a repaint requested while open, listings and readability probes cached
per visit rather than per frame, locked roots left tappable so they can name the permission and the
Settings path, the filter shown as the union of every extension asked for with **"Show all files"**
and a count of the hidden ones, and a save answer of `dir.join(name)` so the app's atomic write finds
its parent.

## 4. The two other crates with no Android backend

- **`arboard`** (the OS clipboard) fails with `cannot find Set in module platform`. It is dropped from
  the Android build entirely (`[target.'cfg(all(not(target_os = "android"), not(target_arch = "wasm32")))'.dependencies]`),
  and the two places that ask for a clipboard take the branch the browser build already takes — the
  snapshot is still taken, the system-wide clipboard is simply out of reach, and the caller already
  says so out loud.
- **`eframe` itself, declared twice.** `accesskit` and `android-native-activity` are mutually
  exclusive in eframe (`compile_error!("`accesskit` feature is only available with
  `android-game-activity`")`), and Cargo takes features as a **union over the whole graph** — so
  `accesskit` has to be gone from *this* crate for the pair never to meet, not merely absent from the
  shell's manifest. `[target.'cfg(not(target_os = "android"))'.dependencies]` keeps it on the desktop.

## 5. Preferences, and the modifier keys the platform never reports

- **`prefs_watch`**: the desktop writes its preferences on the way out; Android never gives that
  moment, so a swipe from Recents took them with it. The shell writes `ui.json` and `prefs.json` from
  the frame loop — change-detected, at most every two seconds, atomically (temp file + rename, so a
  kill mid-write cannot leave truncated JSON).
- **`modifiers.rs`**: winit's Android backend emits only six window events and never
  `ModifiersChanged`; `egui-winit` reads modifiers from that event and nowhere else, and egui carries
  them as an *event*, so `ctx.input(|i| i.modifiers)` is the all-false default for the entire run —
  Ctrl+S, Ctrl+Z and Shift-click are dead. The key events do arrive, so the shell tracks the modifiers
  in `raw_input_hook` and re-injects them (held state, stamped onto every `Key`/`PointerButton` event,
  one `ModifiersChanged` per frame), carrying state **between** frames because the press and the click
  usually land in different ones.
- **`config_changes`** in the APK metadata, so rotation, docking, split-screen resize, density and
  font-scale changes do not recreate the Activity — a recreated Activity destroys the native window
  while the render thread lives on, which is a blank surface.

## 6. No quit hook to wire

PdfCraft has no *File ▸ Exit*; quitting is the window closing (macOS Dock, a Recents swipe, or the
system back gesture). So the stale-`ViewportCommand::Close` problem its siblings fixed does not arise
here — there is no in-app exit path to end the process from. Preferences survive the kill because they
are written while the app runs (§5), and the shell's own save-on-exit is still there for the clean
path.

## 7. Build note: the C toolchain (this is the one app in the set that needs it)

PdfCraft's `pdfcraft-sign` builds `aws-lc-sys` — a C cryptography library behind `aws-lc-rs` — for
every target that is not wasm32. `cc` looks for a bare `clang.exe` and fails; the NDK's `bin`
directory has to be on `PATH` and `CC`/`CXX`/`AR`/`RANLIB` have to name the NDK wrappers for the
target. `scripts/android-env.sh` is that environment, and it is why the build script exists in this
repository rather than a one-liner: `aarch64-linux-android26-clang.cmd` must match
`min_sdk_version = 26`, or the link fails with an API-level error.

## 8. Permissions, icon, theme

`MANAGE_EXTERNAL_STORAGE` (granted by hand under Settings ▸ Special access; Google Play restricts it
to file-manager-class apps, which does not apply to a sideload), `READ_MEDIA_IMAGES` for Android 13+,
and the legacy storage permissions capped with `max_sdk_version`. `res/values/styles.xml` carries the
same two-variant edge-to-edge theme as the siblings — `targetSdkVersion 35` puts the app under a
transparent status bar on Android 15, and winit 0.30 has no safe-area API — and `res/mipmap-*` carries
the launcher icon.

## Upstream files this port changes (22)

| Group | Files | Change |
|---|---|---|
| `crates/ui-egui/Cargo.toml` | 1 | `eframe` declared twice (`accesskit` off for Android), `rfd`/`arboard` moved to a non-Android target table |
| `crates/ui-egui/src/lib.rs` | 1 | `pub mod rfd;` added |
| `crates/ui-egui/src/rfd.rs`, `rfd_android.rs` | 2 | **new** — the name `crate::rfd`: real `rfd` off-Android, host-backed stand-in on it |
| call sites in `actions_ui, combine_ui, comment_props, content_ui, create_ui, editing, export_ui, files, fill_sign, folders_ui, marks_ui, measure_ui, ocr_ui, pickers, print_ui, sign_ui, stamps_ui, zoom_snap` | 18 | `rfd::` → `crate::rfd::`; `arboard` uses `cfg`-gated to the branch the web build already takes |

## 2026-10-13 — Two-finger scrolling (and the wheel Android never sends)

**The complaint:** on the tablet a finger drag selected text instead of moving the page, and there was
no way to scroll at all.

**Why:** every app in this suite scrolls from `egui::Event::MouseWheel` — `WordCraft`'s `scroll.rs` is
explicit about the contract (pixel deltas move the page 1:1, `TouchPhase::End` on the last event is the
"fingers lifted" signal that starts the kinetic coast, wheel notches count as three lines). None of
that runs on Android, because winit's Android backend has no mouse and therefore emits no wheel
events at all (`src/platform_impl/android/mod.rs` sends six window events and a wheel is not one of
them). Touch arrives as `WindowEvent::Touch`, which egui-winit forwards as `egui::Event::Touch` *and*
turns into an emulated mouse pointer — so a finger drags a selection and the page never moves.

**The change — `src/touch_scroll.rs` (new, 231 lines, 3 tests), wired into the shell's
`raw_input_hook`:**

- one finger: untouched, so dragging still selects — the emulated pointer works as before;
- two or more fingers: the centroid's movement becomes `Event::MouseWheel` with
  `MouseWheelUnit::Point` and `TouchPhase::Move`;
- a final `TouchPhase::End` wheel event when the fingers lift, which is what makes the app coast to a
  stop instead of stopping dead;
- while a two-finger gesture is live the emulated **pointer button** events are dropped, or the first
  finger would drag a selection across the page at the same time as it scrolls. Pointer *position*
  events are kept, because the apps only scroll while the pointer is over the canvas;
- the touch events themselves stay in the batch: egui's pinch handling reads them, so a pinch still
  zooms and a two-finger *pan* moves nothing but the page.

**Ctrl+wheel zoom** works too, which is why `modifiers.rs`'s `apply()` now returns the state it tracked
(`egui::Modifiers`) instead of nothing: the bridge stamps it onto the wheel event it synthesises, and
the touch events carry it as well.

**Checked:** the three tests in `touch_scroll.rs` (one-finger pass-through, two-finger centroid →
wheel, the lift event) run on the host with `cargo test -p pdfcraft-android`; the shell compiles for
`aarch64-linux-android`; and the string table of the `.so` inside the released APK carries the module.
On-device behaviour is the porter's tablet test, not an automated one.

## 2026-10-13 — `modifiers.rs` returns the tracked state

`apply(&mut self, raw)` now returns the `egui::Modifiers` it folded into the frame's events. The
synthesised two-finger wheel event has to carry modifiers itself (Ctrl+wheel is zoom in these apps,
plain wheel is scroll), and only the tracker knows them, so the state is handed back rather than
recomputed. Nothing else about the modifier handling changed.
## What is deliberately *not* in the patch

`Cargo.lock` is excluded on purpose: it moves only because the port crate is a new workspace member,
and `cargo` regenerates it on the first build. The patch contains only changes that matter to a reader
— every tracked file the port touches, plus any new file that has to live inside an upstream directory.

## Not verified here

The APK attached to **v0.1.1** was built from exactly this tree on 2026-10-13 (upstream pin + the patch + the port crate), and the checks in the README's verification table were run against it. What that does *not* buy is a runtime claim: it was not launched on a device or emulator as part of publishing, so every behavioural statement below rests on the hand-testing the porter did on an arm64 tablet. The fixes came from
the porter's hand-testing on an arm64 tablet; there is no automated device test here, so "verified" in
the README means the source-side checks listed there plus that testing.
