# Nuva (personal rebuild)

A living notch for Windows: a black pill at the top of your screen that opens on hover and shows
music, synced lyrics, timers, reminders, files and more. Native Rust + Direct2D/DirectComposition,
one ~0.9 MB exe, no web view, no accounts, no tracking. All data stays in your user folder.

Rebuilt from scratch from a feature map of the closed-source Nusa 1.1.0
([docs/nusa-feature-map.md](docs/nusa-feature-map.md)); none of its code is used.

## Install

Run `NusaSetup.exe` to install Nuva.

The bundled setup:
- Installs to `%LOCALAPPDATA%\Programs\Nuva` (no administrator privileges required)
- Automatically closes any running instances before updating
- Creates Start Menu and optional Desktop shortcuts
- Configures autostart with Windows (`HKCU\...\Run`)
- Registers with Windows "Installed apps" with full uninstaller support

Requires the Rust GNU toolchain if building from source (`scoop install rustup-gnu`, then `rustup default stable-x86_64-pc-windows-gnu`).

## Features

Everything from the original:

- **Music** from any player in Windows' media controls (Spotify, Apple Music, browsers, VLC…): art, play/pause, skip, seek
- **Synced lyrics** from LRCLIB: the notch splits in two, the current line is highlighted, and you can click a line to jump to it
- **Focus timer**: Pomodoro with short and long breaks and a chime
- **Water reminders** spread across your active hours, a refill nudge and a 7-day chart
- **To-do** with start/end reminders and snooze
- **Calendar** with month and week views showing your tasks (a "Pro, coming soon" feature in the original)
- **File shelf**: drop files, links or text on the notch, drag them back out, click to open or copy
- **Overlays** for volume, brightness, Caps Lock and input language
- **Popups** for battery, Wi-Fi and Bluetooth headsets
- **Mascot**: eyes appear when you're idle, doze off, and wave when you return
- Hides for full-screen apps, works on any monitor, and has tray menu and settings

New in this build:

- **Clipboard history** (text and images, pin and remove). It skips apps that mark clipboard data as private.
- **Notification mirror**: your Windows notifications (Mail, Outlook, Teams, Discord…) in a page, plus brief popups in the notch
- **System stats**: CPU, GPU, memory, disk and network, with live graphs
- **Lyric line in the pill**: optional, shows the current line while the notch is closed

## Data

| File | What |
|---|---|
| `%LOCALAPPDATA%\Nuva\settings.txt` | settings (`key=value`) |
| `%APPDATA%\Nuva\tasks.txt` | to-do list |
| `%APPDATA%\Nuva\water.txt` | water plan and log |
| `%APPDATA%\Nuva\shelf.txt`, `shelf\` | shelf items and saved snippets |
| `%APPDATA%\Nuva\clipboard.txt` | clipboard history (only if "Remember after restart" is on) |
| `%LOCALAPPDATA%\Nuva\crash.log` | written only if the app crashes |

## Development

```powershell
cargo build                      # debug build
.\target\debug\nuva.exe --preview=calendar   # start expanded on a page (home, lyrics, timer, water, todo, calendar, shelf, clipboard, notifs, stats)
.\target\debug\nuva.exe --settings           # open the settings window
```

| Module | Role |
|---|---|
| `app.rs` | window, input, timers, alerts and wiring |
| `notch.rs` | drawing the pill and pages |
| `settings_window.rs` | settings UI |
| `gfx.rs` | Direct2D/DirectWrite/DirectComposition and the canvas |
| `media.rs`, `lyrics.rs` | now playing and lyrics |
| `pomodoro.rs`, `water.rs`, `todo.rs` | timer and reminders |
| `shelf.rs`, `clipboard.rs`, `notifications.rs`, `stats.rs` | the other pages |
| `system.rs` | volume, keyboard, battery, network, Bluetooth, monitors, full-screen detection |
| `tray.rs` | tray icon |
