# Nusa 1.1.0 — feature map (static analysis)

Source: `NusaSetup.exe` (sha256 `e1995b7f…c8021`), unpacked offline with a custom Inno Setup 6.7
reader; `nusa.exe` (sha256 `7ef5d186…7c4d5`, verified against the installer's own hash) analyzed
with REA 6.8.0 (`inspect-artifact`, `inspect-managed-artifact`, `inspect-pe-imports`,
`inspect-rust-binary`, `inspect-pe-resources`) plus string extraction. Nothing was executed.

## Build / runtime
- Rust (rustc commit `48a229ce…`), `windows` crate 0.58, x86-64, single 1 MB exe.
- Rendering: Direct2D + DirectWrite on D3D11, presented through DirectComposition
  (transparent, GPU-composited top-of-screen window, class `NotchWindow`). Redraws only on change.
- Single instance: mutex `Local\NusaNotch.SingleInstance`.
- Per-monitor DPI aware (`SetProcessDpiAwarenessContext`, `GetDpiForMonitor`).
- Hides for full-screen apps (`SHQueryUserNotificationState`).
- Tray icon (`Shell_NotifyIconW`, re-adds on `TaskbarCreated`) with menu: Settings…, Start with
  Windows, Nusa Mascot, Minimal notch, Notch size, Main display / Display N, About Nusa.
- Separate settings window (`NusaSettings`, launched via `--settings`).
- Autostart: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Nusa`.

## Data (plain text key=value files)
- `%LOCALAPPDATA%\Nusa\settings.txt`
- `%APPDATA%\Nusa\water.txt` (`bottle=`, `goal=`, `start=`, `hours=`, `taken=`, `fired=`)
- `tasks.txt` / `todo.txt`, `shelf.txt`
- Uninstaller offers to remove data ("to-do list, water log, shelf, settings").

### Settings keys
`size`, `display`, `pet`, `minimal`, `sound`, `lyrics`, `todo_start`, `todo_end`, `snooze`,
`week_monday`, `focus`, `short_break`, `long_break`, `hud_volume`, `hud_brightness`, `hud_caps`,
`hud_language`, `popup_battery`, `popup_wifi`, `popup_bluetooth`, `open_click`, `page_icons`,
`fast_animation`, `shelf_clear`, `calendar_week`, `hide_fullscreen`

## Notch behaviour
- Collapsed pill at the top centre of the chosen display; opens on hover (or click, setting).
- Sizes: Auto / 100% / 115% / 130% / 150% (restart to apply). Labels Small…Extra large.
- "Minimal notch": smaller pill when nothing is playing.
- Pages with page dots (optional page icons on hover). Animation speed Normal / Fast.
- Mascot ("companion mode"): eyes appear when idle, doze after a while, wave on return
  (`GetLastInputInfo`).

## Pages / modules
1. **Media** — `GlobalSystemMediaTransportControlsSessionManager`: title/artist/art, play/pause,
   skip, seek, for any SMTC player (Spotify, browsers, Apple Music…).
2. **Lyrics** — `https://lrclib.net/api/search?track_name=…&artist_name=…`, User-Agent
   `Nusa/1.1.0 (+https://rama2555.github.io/nusa)`. Synced LRC; splits notch in two, highlights the
   current line, click a line to seek. States: "Loading lyrics", "Lyrics not available".
3. **Pomodoro** — Focus / Short break / Long break durations (defaults 25 / 5 / 15 min), countdown in
   the notch, chime (`PlaySoundW`). Popups: "Focus Time!", "Break Time!", "Long Break!".
4. **Water** — bottle size, daily goal (1.5 / 2 / 2.5 / 3 L), active from/until hours, reminders
   spread across active hours, "Time to Drink" / "Time to Refill", buttons Drank / Refilled / Snooze,
   today's progress chart.
5. **To-do** — tasks with a time; reminders "When a task starts" / "When time is up"; snooze
   5/10/15 min; "Task Time!" (Later / Start), "Time's Up!"; Add/Edit task, "Done Today".
6. **Calendar** — month/week view, week starts Sunday/Monday (preview; "coming with Nusa Pro").
7. **File shelf** — drop files onto the notch (`RegisterDragDrop`), drag them back out
   (`DoDragDrop`, `SHCreateDataObject`), click to open or copy, Select / Remove / Clear all; also
   accepts URLs (`UniformResourceLocatorW`, writes `.url` shortcuts) and text snippets
   (`.nusatext`); optional "start with an empty shelf".

## HUDs & popups
- HUD: volume, brightness, Caps Lock, input language (keyboard hook + `GetKeyboardLayout`).
- Popups: battery (low / critical / fully charged / charger connected / unplugged),
  Wi-Fi (`NetworkInformation`: connected / no internet / disconnected),
  Bluetooth headset connect (`BluetoothDevice`, `cfgmgr32` BTHENUM, audio-sink UUIDs 110B/111E/1108).
- `UserNotificationListener` is referenced (notification access).

## "Pro" / unfinished in 1.1.0
- Calendar ("Calendar is coming with Nusa Pro"), "Nusa Pro — Coming soon".
