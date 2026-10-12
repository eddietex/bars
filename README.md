<p align="center"><img src="packaging/linux/icons/128x128/xyz.hink.Bars.png" width="96" alt="Bars"></p>

# Bars

Private, on-device voice dictation that works in every app. Hold a shortcut,
speak, let go: the text appears wherever your cursor is. Nothing leaves your
computer.

Bars is built in Rust with [GPUI](https://www.gpui.rs) (Zed's UI framework,
via [GPUI Kit](https://gpui-kit.com)) and runs NVIDIA's Parakeet speech models
on the CPU with ONNX Runtime.

## Using it

| Gesture | What happens |
| --- | --- |
| **Hold** `Ctrl` + `Super` | Talk; release to insert the text |
| **Double-tap** `Ctrl` + `Super` | Hands-free: keep talking, tap again to insert |
| `Esc` while recording | Cancel |
| Tray menu → *Paste last transcript* | Re-insert the last dictation |
| `bars --toggle` | Start/stop from any custom key binding or script |

The shortcut, activation style (hold + double-tap / hold / toggle), microphone,
insertion method, model and more are in the settings window (click the tray
icon). Say **"new line"** or **"new paragraph"** to break lines; filler words
("um", "uh") are removed; the Dictionary page holds word replacements and
spoken snippets.

## Speech models

All models run locally on the CPU (int8 ONNX). Measured on an 11 s clip:

| Model | Download | Languages | Ryzen 9950X3D | 2 cores |
| --- | --- | --- | --- | --- |
| **Parakeet TDT 0.6B v3** (default) | 671 MB | 25 European, auto-detect | 245 ms | 513 ms |
| Parakeet TDT 0.6B v2 | 661 MB | English | ≈ v3 | ≈ v3 |
| Canary 180M Flash | 213 MB | en, de, es, fr | 275 ms | 582 ms |

Models download from Hugging Face on first run into
`~/.local/share/bars/models` (Flatpak: `~/.var/app/xyz.hink.Bars/data`).
Adding a model is a catalog entry in `crates/bars-core/src/models.rs`; any
engine supported by [transcribe-rs](https://github.com/cjpais/transcribe-rs)
(Parakeet, Canary, Moonshine, SenseVoice, Whisper...) can be wired in.

## How it reaches other apps (Linux)

Bars picks the best available backend and falls back automatically:

| Need | Preferred | Fallback (no setup) |
| --- | --- | --- |
| Global shortcut | **evdev**: reads keyboards; any chord, including modifier-only `Ctrl+Super` | **GlobalShortcuts portal**: desktop owns the binding (KDE, GNOME 48+, Hyprland) |
| Insert text | **uinput** virtual keyboard: paste or layout-aware typing | **RemoteDesktop portal**: asks once, remembered |
| Clipboard paste | Wayland data-control, with the old clipboard restored | Type key by key |
| Recording pill | **layer-shell** overlay that never takes focus | Unfocused popup window |

The preferred backends need read access to keyboards and write access to
`/dev/uinput`. Bars requests this with a small udev rule that grants it to
the user logged in at the seat (systemd `uaccess`), with no group changes and
no reboot. The **setup assistant** opens on first launch and walks through
the microphone, this permission (one click: "Allow access" shows the
system's password prompt; from Flatpak it goes through the host), the model
download and a first dictation. If you'd rather not change system settings,
it offers the portal route instead. When Bars autostarts with something
missing, it posts a notification rather than opening a window. Manual route:

```sh
pkexec ./scripts/install-udev-rule.sh
```

Note: like membership in the `input` group, this lets programs running as you
read keyboard events.

The default paste shortcut is **Shift+Insert**, with the text placed on both the
clipboard and the primary selection, so it works in GUI apps and terminals
alike. Ctrl+V and Ctrl+Shift+V are available too.

## Building

Development happens in a toolbox/distrobox with Rust, a C compiler and the
usual headers (`wayland-devel libxkbcommon-devel alsa-lib-devel
fontconfig-devel vulkan-loader-devel`).

```sh
cargo run -p bars                       # debug build
cargo test -p bars-core                 # core unit tests
cargo run --release -p bars-core --example bench -- testdata/jfk.wav   # model speed
cargo run -p bars-core --example headless   # controller without UI
```

### Flatpak (recommended distribution)

```sh
flatpak install --user flathub org.flatpak.Builder org.freedesktop.Sdk.Extension.rust-stable//26.08
./scripts/build-flatpak.sh               # builds + installs for the current user
BUNDLE=1 ./scripts/build-flatpak.sh      # also writes bars.flatpak
flatpak run xyz.hink.Bars
```

Why Flatpak over AppImage: it's the native app format on image-based distros
(Silverblue, Kinoite, Bazzite, Aurora...), works the same across distros
without glibc/FUSE issues, updates incrementally, and the code runs natively.
ONNX Runtime is linked statically and rendering uses Vulkan, so there's no
performance penalty.

## Releases

`.github/workflows/release.yml` builds every platform when a `v*` tag is
pushed and attaches the results to a draft GitHub Release:

| Platform | Artifact |
| --- | --- |
| Linux | `bars-{x86_64,aarch64}.flatpak` (any distro) and `bars-linux-{x86_64,aarch64}.tar.gz` (glibc 2.38+: Fedora 39+, Ubuntu 24.04+, Debian 13+) |
| Windows | NSIS installer for x64 and arm64 (`.exe`, per-user, no admin needed) |
| macOS | `.dmg` (Apple Silicon; ONNX Runtime no longer ships Intel Mac builds) |

```sh
git tag v0.1.0 && git push origin v0.1.0
```

Windows and macOS builds are unsigned for now, so SmartScreen and Gatekeeper
warn on first launch, and macOS may ask for Input Monitoring and
Accessibility again after an update. Signing needs a code-signing
certificate (Windows) and a Developer ID with notarization (macOS).

CI (Actions > CI > Run workflow) also builds Windows and macOS and runs
`scripts/smoke`: the app is started on a real runner, driven through its
command line, and screenshots plus logs are uploaded.

## Layout

```
crates/bars-core   engine, audio, hotkeys, text injection, config, history
  src/controller.rs   single-threaded state machine; UI talks to it by message
  src/hotkey.rs       chords + push-to-talk/double-tap recognizer (tested)
  src/engine.rs       model worker thread (transcribe-rs / ONNX Runtime)
  src/audio.rs        cpal capture (PulseAudio/PipeWire), resampling, cues
  src/text.rs         filler removal, dictionary, snippets, voice commands
  src/platform/       OS backends: linux/ (evdev, uinput, portals, clipboard),
                      win/ (keyboard hook), mac/ (event tap, TCC permissions),
                      native_inject.rs (Windows/macOS paste + typing)
crates/bars        GPUI app: tray, overlay pill, settings window
  src/overlay_native.rs  Windows/macOS pill: click-through, active screen, blur (macOS)
  src/ipc_*.rs          single instance + CLI (D-Bus on Linux, local socket elsewhere)
third_party/        gpui-pre-linux with a Wayland frame-waker fix (see patch)
packaging/          .desktop, AppStream, icons (from artwork/), udev rule, Flatpak manifest, macOS Info.plist
scripts/smoke/      Windows/macOS runtime smoke tests run by CI
```

The shortcut, insertion, overlay, tray, autostart and permission backends
are implemented on all three platforms. What the setup assistant asks for
lives in `platform/requirements.rs`: keyboard access on Linux (udev rule),
microphone privacy on Windows, and Microphone, Input Monitoring and
Accessibility on macOS.

Dev helpers: `BARS_ONBOARDING=<welcome|microphone|access|model|try|done>`
opens the assistant at a step; `BARS_DEV_NO_ACCESS=1` previews it as a user
without keyboard access; `BARS_PAGE=<home|general|model|...>` opens settings
at a page.
