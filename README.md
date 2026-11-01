# Nuva

A living notch for Windows: a black pill at the top of your screen that opens on hover and shows
music, synced lyrics, timers, reminders, files and more. Native Rust + Direct2D/DirectComposition,
one ~0.9 MB exe, no web view, no accounts, no tracking. All data stays in your user folder.

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

- **Dedicated Calculator Page & Command Palette**: Interactive 5×5 on-screen keypad (`0-9`, `.`, `+`, `-`, `×`, `÷`, `^`, `%`, `√`, `sin`, `cos`, `π`, `(`, `)`, `C`, `=`), live equation input with evaluated result (`= 110`), and direct physical keyboard typing. Also converts units (`50 km to mi`, `32 c to f`), runs quick math, and launches apps (`calc`, `wt`, `notepad`).
- **Page Management in Settings**: A dedicated "Pages" section in Settings lets you toggle any page on or off (Now Playing, Focus Timer, Water, To-do, Calendar, Shelf, Clipboard, Notifications, System Stats, Calculator). Disabled pages are completely hidden from the notch's page dots and swipe gestures.
- **Microphone Privacy Dot & 1-Click Mute**: Real-time orange indicator dot when any app uses your microphone (via Windows `ConsentStore`); click the header mic button to toggle system mic mute with instant toast feedback.
- **Download Progress**: Subtle glow progress bar along the notch bottom edge when downloads are active (`.crdownload`, `.part`, `.download`).
- **1-Click Audio Output Switcher**: Instant cycle between speakers, headphones, and Bluetooth earbuds from the notch header without opening Windows Sound settings.
- **Screen Color Picker / Eyedropper**: One click on the eyedropper icon lets you click any pixel on any monitor/app to copy its HEX `#RRGGBB` directly to your clipboard and Clipboard History (powered by low-level hook `WH_MOUSE_LL`). Right-click or press Esc to cancel.
- **Scroll-on-pill Volume Gesture with Cooldown**: Scroll the mouse wheel on the collapsed notch to change system volume. Keeps the notch in small pill mode and prevents accidental hover expansion until 2 seconds after you finish scrolling.
- **Quick File Actions**: Hover any dropped file on the Shelf to copy path, convert image (PNG ↔ JPG), compress to ZIP, or reveal in Explorer.
- **Screen Capture Exclusion**: Hides the notch window from screen sharing, recordings, and screenshots (Zoom, Teams, Discord, OBS Studio) via `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`.
- **Live Activity API & CLI**: Push tasks, notifications, and progress from terminal scripts or CI via named pipe `\\.\pipe\nuva_notch`:
  ```powershell
  nuva --activity "Building project" --progress 75
  nuva --notify "Build Passed" --sub "All 42 tests green"
  nuva --clear-activity
  ```
- **Header Tools Customization**: Under Settings > HUD & Popups > Header Tools, toggle individual buttons for Color Picker, Calculator/Search, Audio Switcher, and Mic Privacy.
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
.\target\debug\nuva.exe --preview=calc   # start expanded on a page (home, lyrics, timer, water, todo, calendar, shelf, clipboard, notifs, stats, calc)
.\target\debug\nuva.exe --settings       # open the settings window
```

| Module | Role |
|---|---|
| `app.rs` | window, input, timers, alerts and wiring |
| `cmd.rs` | command palette, calculator math engine and unit converter |
| `notch.rs` | drawing the pill and pages |
| `settings_window.rs` | settings UI |
| `gfx.rs` | Direct2D/DirectWrite/DirectComposition and the canvas |
| `media.rs`, `lyrics.rs` | now playing and lyrics |
| `pomodoro.rs`, `water.rs`, `todo.rs` | timer and reminders |
| `shelf.rs`, `clipboard.rs`, `notifications.rs`, `stats.rs` | the other pages |
| `system.rs` | volume, keyboard, battery, network, Bluetooth, monitors, full-screen detection |
| `tray.rs` | tray icon |
