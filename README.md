# Nuva

A living notch for Windows: a black pill at the top of your screen that opens on hover and shows
music, synced lyrics, timers, reminders, files and more. Native Rust + Direct2D/DirectComposition,
a single self-contained exe (~1.6 MB), no web view, no accounts, no tracking. All data stays in your user folder.

## Install

Run `NuvaSetup.exe` to install Nuva.

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
- Hides for full-screen apps, works on any monitor, and has tray menu and settings

New in this build:

- **4-Zone Dropzone Action Bar**: Dragging files toward the top of your screen expands the notch into 4 interactive drop targets:
  - **Hold**: Instant file shelf storage for reference and drag-out.
  - **Convert**: PNG ↔ JPG in-memory WIC conversion (flattening transparency over white for JPEGs).
  - **Compress**: Native Deflate packaging into a `.zip` archive without external tools. Folders are added recursively with depth and symlink guards, duplicate names are disambiguated, and partial archives are removed on failure.
  - **Extract OCR**: 100% offline, on-device WinRT `Windows.Media.Ocr` text extraction directly to your clipboard.
- **Dynamic Album Art Palette Extraction**: Automatically samples dominant vibrant colors from active music artwork to theme the media visualizer, waveform, and scrubbers, using a 12-bucket hue histogram and HSV value clamping.
- **5-Band Real-Time Audio Visualizer**: Streams loopback audio from the default Windows render endpoint using a 5-point Goertzel filter bank (65 Hz, 180 Hz, 520 Hz, 1.8 kHz, 5.8 kHz) with fast-attack / slow-release envelope smoothing. Zero idle CPU: capture thread parks on a condition variable when playback stops.
- **Desktop Auto-Hide & Top-Edge Hover**: Automatically conceals the notch when windows are active, but stays visible on clean desktop screens and reveals instantly when your cursor brushes the top 4 pixels of the display.
- **Dedicated Calculator Page**: Interactive 5×5 on-screen keypad (`0-9`, `.`, `+`, `-`, `×`, `÷`, `^`, `%`, `√`, a trig key, `π`, `Ans`, `C`, `=`), live equation input with evaluated result (`= 531`), `Ans` variable memory, natural percentage math (`450 + 18%`, `20% of 500`), tap the trig key to cycle `sin` → `cos` → `tan`, and direct physical keyboard typing with 1-click copy.
- **Search & Universal Launcher**: Spotlight-style command palette (`/` or `Ctrl+K`) with `↑`/`↓` and `Tab` keyboard navigation. Launch apps from the Start Menu catalogue (`notepad`, `wt`, `chrome`, `code`), open the in-app calculator with `calc`, run system commands with confirmation (`lock`, `sleep`, `restart`, `shutdown`), web search shortcuts (`g <query>`, `yt <query>`, `gh <query>`, `w <query>`), unit conversions (`50 km to mi`, `100 c to f`), date calculations (`days until 25 dec`, `days to new year`), and instant navigation.
- **Input Passthrough Outside Notch Geometry**: Native `WM_NCHITTEST` returning `HTTRANSPARENT` whenever cursor coordinates fall outside the animated spring bounds (`sw.v`, `sh.v` + flare), allowing underlying browser tabs and titlebars to receive clicks seamlessly.
- **Native WIC Image Conversion**: Zero-process, asynchronous image transcoding on the Shelf via Windows Imaging Component (`IWICImagingFactory`), converting PNG ↔ JPG in under 5 ms with zero PowerShell overhead.
- **Image Clipboard History**: Keeps text and full-resolution CF_DIB image entries in memory alongside a bounded thumbnail cache (images are capped so a large screenshot burst cannot grow memory without limit). Text history can persist across restarts.
- **Page Order Changer & Visibility in Settings**: A dedicated "Pages" section in Settings lets you reorder pages with `▲` and `▼` buttons and toggle any page on or off. Disabled pages are completely hidden from the notch's page dots and gestures; active pages strictly follow your custom order. A "Reset Order" button restores defaults at any time.
- **Download Progress**: Subtle glow progress bar along the notch bottom edge when downloads are active (`.crdownload`, `.part`, `.download`).
- **Screen Color Picker / Eyedropper**: One click on the eyedropper icon in the top right header lets you click any pixel on any monitor/app to copy its HEX `#RRGGBB` directly to your clipboard and Clipboard History (powered by low-level hook `WH_MOUSE_LL`). Right-click or press Esc to cancel.
- **Scroll-on-pill Volume Gesture with Cooldown**: Scroll the mouse wheel on the collapsed notch to change system volume. Keeps the notch in small pill mode and prevents accidental hover expansion until 2 seconds after you finish scrolling.
- **Quick File Actions**: Hover any dropped file on the Shelf to copy path, convert image (PNG ↔ JPG), compress to ZIP, or reveal in Explorer.
- **Screen Capture Exclusion**: Hides the notch window from screen sharing, recordings, and screenshots (Zoom, Teams, Discord, OBS Studio) via `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`.
- **Live Activity API & CLI**: Push tasks, notifications, and progress from terminal scripts or CI via secure named pipe `\\.\pipe\nuva_notch`:
  ```powershell
  nuva --activity "Building project" --progress 75 --sub "Compiling crates"
  nuva --notify "Build Passed" --sub "All 42 tests green"
  nuva --clear-activity
  ```
- **Clipboard history** (text and images, pin and remove). It skips apps that mark clipboard data as private.
- **Notification mirror**: your Windows notifications (Mail, Outlook, Teams, Discord…) in a page, plus brief popups in the notch
- **System stats**: CPU, GPU, memory, disk and network, with live graphs
- **Lyric line in the pill**: optional, shows the current line while the notch is closed

## Performance & Overhead

- **Idle CPU**: **<0.1%** measured via Windows Performance Monitor (`typeperf "\Process(nuva)\% Processor Time"`). When collapsed and idle there is no render loop, expensive microphone/download probes are throttled, and the audio loopback thread completely parks on a synchronization condition variable when playback stops.
- **Memory Footprint**: ~15–18 MB working set, trimmed automatically on startup via `SetProcessWorkingSetSize`.
- **Image Conversion & Shelf Latency**: <5 ms average measured via `std::time::Instant` over 100 consecutive WIC runs (eliminating the 300–800 ms cold-start latency of PowerShell).

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
.\target\debug\nuva.exe --preview=calc   # start expanded on a page (home, timer, water, todo, calendar, shelf, clipboard, notifs, stats, calc)
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
