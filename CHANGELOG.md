# Changelog

## v0.1.1 — 2026-10-13 (scrolling, modifiers, fonts, picker wording)

| | |
|---|---|
| APK | `pdfcraft-0.6.0-arm64-v8a-20261013.apk` (21.8 MB) — supersedes the v0.1.0 asset, same signing key |
| sha256 | `e6e77cf7b7516bbb4b9f297b51b31b3fec37b0cba1813e57bce9c98015936c40` |
| Built from | upstream `storytold/pdfcraft` @ `3d2bdfe71ada10f6bc39afc680dce61bbf080cbc` + this repository at the commit that released it |
| Package | `ai.storyteller.pdfcraft` — versionName `0.6.0`, versionCode `16778752`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |

### Changed
- **Two-finger scrolling.** A two-finger drag now scrolls the document (synthesised `MouseWheel`
  events from the touch centroid, with the lift event that starts the kinetic coast); one finger still
  selects and a pinch still zooms.
- **Ctrl+wheel zoom** — the shell's modifier tracker now hands its state to the touch-to-wheel bridge,
  which stamps it onto the wheel events it synthesises.

### Notes
The build was made from exactly the source in this repository (upstream pin + the patch + the port
crate) and signed with the same local test key as v0.1.0, so it installs over it.


## v0.1.0 — first published port (PdfCraft 0.6.0)

| | |
|---|---|
| APK | `pdfcraft-0.6.0-arm64-v8a-v3.apk` (21.8 MB) |
| sha256 | `6c03616ce94caf0a06532e6eac46d7e975a3483afe5baec05cdf4878cd80ad18` |
| Built from | upstream `storytold/pdfcraft` @ `3d2bdfe71ada10f6bc39afc680dce61bbf080cbc` + `patches/0001-pdfcraft-android-port.patch` + `apps/pdfcraft-android/` |
| Package | `ai.storyteller.pdfcraft` — versionName `0.6.0`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | throwaway local test key, not an official key |

### In this release
- Android shell crate: NativeActivity entry point, `Services` implementation, file logger, preferences
  written while the app runs (atomic), modifier-key synthesis, Activity not recreated on configuration
  change.
- **Every file dialog works**: 42 call sites moved to `crate::rfd`, which is the real `rfd` on the
  desktop and the host's own egui picker on Android — Open, Save As, export folders, OCR folders,
  attach, sign, stamps, print.
- Clipboard code that cannot exist on Android is compiled out rather than left broken; the paths that
  remain say what they can and cannot do instead of failing at runtime.
- In-app picker with cached listings, "Show all files", hidden-file count, self-explaining locked
  roots, and a chosen save path returned to the app so the document learns which file it is.
- Launcher icon, storage permissions, edge-to-edge theme variant.


### Known issues
See *Known gaps* in the [README](README.md) and [`docs/PORT-NOTES.md`](docs/PORT-NOTES.md).
