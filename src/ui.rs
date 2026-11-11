//! Everything clickable in the notch and the settings window.

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Page(usize),
    // Media
    PlayPause,
    Next,
    Prev,
    SeekBar,
    ToggleLyrics,
    LyricSeek(f64),
    // Pomodoro
    PomoStartPause,
    PomoReset,
    PomoSkip,
    PomoMode(u8),
    // Water
    WaterDrank,
    WaterRefilled,
    WaterUndo,
    // To-do
    TodoToggle(u64),
    TodoDelete(u64),
    TodoEdit(u64),
    TodoAdd,
    TodoSave,
    TodoCancel,
    TodoField,
    TodoStart(i32),
    TodoDur(i32),
    TodoDay(i32),
    // Calendar
    CalShift(i32),
    CalView(bool),
    CalDay(i64),
    // Shelf
    ShelfItem(usize),
    ShelfRemove(usize),
    ShelfClear,
    ShelfCopyPath(usize),
    ShelfConvert(usize),
    ShelfZip(usize),
    ShelfExplore(usize),
    #[allow(dead_code)]
    SetDropAction(crate::shelf::DropAction),
    // Features: Audio, Mic, Color Picker, Command, Calculator
    ToggleMicMute,
    CycleAudioDevice,
    PickColor,
    OpenCommandPalette,
    CommandRun,
    CommandConfirm(String),
    CalcInput(&'static str),
    CalcClear,
    CalcBackspace,
    CalcTrig,
    // Clipboard
    ClipCopy(usize),
    ClipRemove(usize),
    ClipPin(usize),
    ClipClear,
    // Notifications
    NotifRemove(usize),
    NotifClear,
    // Alerts
    AlertButton(usize),
    // Window
    OpenSettings,
    // Settings window
    SetSection(usize),
    SetToggle(&'static str),
    SetNum(&'static str, u32),
    SetStep(&'static str, i32),
    SetStartup,
    SetDisplay(u32),
    WaterGoal(u32),
    WaterStep(&'static str, i32),
    OpenUrl(&'static str),
    OpenDataFolder,
    // Page ordering
    PageMoveUp(usize),
    PageMoveDown(usize),
    PageResetOrder,
}
