//! Command palette, quick math calculator, and unit converter.

pub struct CommandPalette {
    pub query: String,
    pub result: Option<CommandItem>,
    pub active: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct CommandItem {
    pub title: String,
    pub sub: String,
    pub icon: char,
    pub kind: CommandKind,
}

#[derive(Clone, Debug)]
pub enum CommandKind {
    Copy(String),
    Launch(String),
    Page(usize),
    Action(crate::ui::Action),
}

impl CommandPalette {
    pub fn new() -> Self {
        CommandPalette {
            query: String::new(),
            result: None,
            active: false,
        }
    }

    #[allow(dead_code)]
    pub fn set_query(&mut self, q: String) {
        self.query = q;
        self.update();
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.update();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.update();
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.result = None;
    }

    pub fn update(&mut self) {
        let q = self.query.trim();
        if q.is_empty() {
            self.result = None;
            return;
        }

        // 1. Try Unit Conversion (e.g. "50 km to mi", "100 c to f")
        if let Some(res) = eval_units(q) {
            self.result = Some(res);
            return;
        }

        // 2. Try Math Evaluation (e.g. "124 * 1.18", "sqrt(144) + 10")
        if let Some(res) = eval_math(q) {
            self.result = Some(res);
            return;
        }

        // 3. Try App Launch / Navigation / Actions
        if let Some(res) = eval_command(q) {
            self.result = Some(res);
            return;
        }

        self.result = Some(CommandItem {
            title: format!("Search \"{q}\""),
            sub: "Press Enter to search with default browser".into(),
            icon: crate::gfx::ico::GLOBE,
            kind: CommandKind::Launch(format!("https://www.google.com/search?q={}", urlencoding(q))),
        });
    }
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

// ---------------------------------------------------------------- math evaluator

fn eval_math(input: &str) -> Option<CommandItem> {
    // Check if input looks like math (contains digits and at least one math operator or func)
    let has_digit = input.chars().any(|c| c.is_ascii_digit());
    if !has_digit { return None; }
    let s = input.replace(' ', "").to_lowercase();
    let has_op = s.contains('+') || s.contains('-') || s.contains('*') || s.contains('/')
        || s.contains('^') || s.contains('%') || s.contains("sqrt") || s.contains("sin")
        || s.contains("cos") || s.contains("tan") || s.contains("abs");
    if !has_op && !s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }

    let mut parser = Parser::new(&s);
    let val = parser.parse_expr().ok()?;
    if !parser.is_eof() { return None; }

    let formatted = if val.fract().abs() < 1e-9 {
        format!("{}", val as i64)
    } else {
        format!("{:.4}", val).trim_end_matches('0').trim_end_matches('.').to_string()
    };

    Some(CommandItem {
        title: format!("= {}", formatted),
        sub: format!("{input} • Press Enter to copy"),
        icon: crate::gfx::ico::CALCULATOR,
        kind: CommandKind::Copy(formatted),
    })
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser { chars: s.chars().peekable() }
    }

    fn is_eof(&mut self) -> bool {
        self.chars.peek().is_none()
    }

    fn parse_expr(&mut self) -> Result<f64, ()> {
        let mut a = self.parse_term()?;
        while let Some(&c) = self.chars.peek() {
            if c == '+' {
                self.chars.next();
                a += self.parse_term()?;
            } else if c == '-' {
                self.chars.next();
                a -= self.parse_term()?;
            } else {
                break;
            }
        }
        Ok(a)
    }

    fn parse_term(&mut self) -> Result<f64, ()> {
        let mut a = self.parse_power()?;
        while let Some(&c) = self.chars.peek() {
            if c == '*' {
                self.chars.next();
                a *= self.parse_power()?;
            } else if c == '/' {
                self.chars.next();
                let b = self.parse_power()?;
                if b.abs() < 1e-12 { return Err(()); }
                a /= b;
            } else if c == '%' {
                self.chars.next();
                let b = self.parse_power()?;
                if b.abs() < 1e-12 { return Err(()); }
                a %= b;
            } else {
                break;
            }
        }
        Ok(a)
    }

    fn parse_power(&mut self) -> Result<f64, ()> {
        let a = self.parse_factor()?;
        if let Some(&'^') = self.chars.peek() {
            self.chars.next();
            let b = self.parse_power()?;
            Ok(a.powf(b))
        } else {
            Ok(a)
        }
    }

    fn parse_factor(&mut self) -> Result<f64, ()> {
        if let Some(&'-') = self.chars.peek() {
            self.chars.next();
            return Ok(-self.parse_factor()?);
        }
        if let Some(&'+') = self.chars.peek() {
            self.chars.next();
            return self.parse_factor();
        }
        if let Some(&'(') = self.chars.peek() {
            self.chars.next();
            let v = self.parse_expr()?;
            if self.chars.next() != Some(')') { return Err(()); }
            return Ok(v);
        }

        // Functions / Constants
        let mut name = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_alphabetic() {
                name.push(c);
                self.chars.next();
            } else {
                break;
            }
        }

        if !name.is_empty() {
            if name == "pi" { return Ok(std::f64::consts::PI); }
            if name == "e" { return Ok(std::f64::consts::E); }
            if let Some(&'(') = self.chars.peek() {
                self.chars.next();
                let arg = self.parse_expr()?;
                if self.chars.next() != Some(')') { return Err(()); }
                return match name.as_str() {
                    "sqrt" => if arg >= 0.0 { Ok(arg.sqrt()) } else { Err(()) },
                    "abs" => Ok(arg.abs()),
                    "sin" => Ok(arg.sin()),
                    "cos" => Ok(arg.cos()),
                    "tan" => Ok(arg.tan()),
                    "floor" => Ok(arg.floor()),
                    "ceil" => Ok(arg.ceil()),
                    _ => Err(()),
                };
            }
            return Err(());
        }

        // Number
        let mut num_str = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                num_str.push(c);
                self.chars.next();
            } else {
                break;
            }
        }

        if let Ok(v) = num_str.parse::<f64>() {
            Ok(v)
        } else {
            Err(())
        }
    }
}

// ---------------------------------------------------------------- unit converter

fn eval_units(input: &str) -> Option<CommandItem> {
    let lower = input.to_lowercase();
    let parts: Vec<&str> = lower.split_whitespace().collect();
    if parts.len() != 4 || (parts[2] != "to" && parts[2] != "in") {
        return None;
    }

    let val = parts[0].parse::<f64>().ok()?;
    let from = parts[1];
    let to = parts[3];

    let (converted, unit_label) = convert_unit(val, from, to)?;
    let formatted = if converted.fract().abs() < 1e-9 {
        format!("{converted} {unit_label}")
    } else {
        format!("{:.4} {}", converted, unit_label).trim_end_matches('0').trim_end_matches('.').to_string()
    };

    Some(CommandItem {
        title: format!("= {formatted}"),
        sub: format!("{input} • Press Enter to copy"),
        icon: crate::gfx::ico::REFRESH,
        kind: CommandKind::Copy(formatted),
    })
}

fn convert_unit(v: f64, from: &str, to: &str) -> Option<(f64, &'static str)> {
    match (from, to) {
        // Distance
        ("km", "mi") | ("km", "miles") => Some((v * 0.621371, "mi")),
        ("mi", "km") | ("miles", "km") => Some((v / 0.621371, "km")),
        ("m", "ft") | ("meters", "ft") => Some((v * 3.28084, "ft")),
        ("ft", "m") | ("feet", "m") => Some((v / 3.28084, "m")),
        ("cm", "in") | ("cm", "inch") | ("cm", "inches") => Some((v * 0.393701, "in")),
        ("in", "cm") | ("inch", "cm") | ("inches", "cm") => Some((v / 0.393701, "cm")),
        ("km", "m") => Some((v * 1000.0, "m")),
        ("m", "km") => Some((v / 1000.0, "km")),
        // Weight
        ("kg", "lb") | ("kg", "lbs") => Some((v * 2.20462, "lbs")),
        ("lb", "kg") | ("lbs", "kg") => Some((v / 2.20462, "kg")),
        ("g", "oz") => Some((v * 0.035274, "oz")),
        ("oz", "g") => Some((v / 0.035274, "g")),
        // Temperature
        ("c", "f") => Some((v * 9.0 / 5.0 + 32.0, "°F")),
        ("f", "c") => Some(((v - 32.0) * 5.0 / 9.0, "°C")),
        // Digital storage
        ("gb", "mb") => Some((v * 1024.0, "MB")),
        ("mb", "gb") => Some((v / 1024.0, "GB")),
        ("tb", "gb") => Some((v * 1024.0, "GB")),
        ("gb", "tb") => Some((v / 1024.0, "TB")),
        ("mb", "kb") => Some((v * 1024.0, "KB")),
        // Time
        ("hours", "min") | ("h", "m") | ("h", "min") => Some((v * 60.0, "min")),
        ("min", "hours") | ("m", "h") | ("min", "h") => Some((v / 60.0, "hours")),
        ("days", "hours") | ("d", "h") => Some((v * 24.0, "hours")),
        ("hours", "days") | ("h", "d") => Some((v / 24.0, "days")),
        _ => None,
    }
}

// ---------------------------------------------------------------- command launcher

fn eval_command(q: &str) -> Option<CommandItem> {
    let lower = q.to_lowercase();
    match lower.as_str() {
        "calc" | "calculator" => Some(CommandItem {
            title: "Open Windows Calculator".into(),
            sub: "Launch calc.exe".into(),
            icon: crate::gfx::ico::CALCULATOR,
            kind: CommandKind::Launch("calc.exe".into()),
        }),
        "notepad" | "notes" => Some(CommandItem {
            title: "Open Notepad".into(),
            sub: "Launch notepad.exe".into(),
            icon: crate::gfx::ico::EDIT,
            kind: CommandKind::Launch("notepad.exe".into()),
        }),
        "cmd" | "terminal" | "powershell" => Some(CommandItem {
            title: "Open Terminal".into(),
            sub: "Launch wt.exe or powershell.exe".into(),
            icon: crate::gfx::ico::KEYBOARD,
            kind: CommandKind::Launch("wt.exe".into()),
        }),
        "explorer" | "files" => Some(CommandItem {
            title: "Open File Explorer".into(),
            sub: "Launch explorer.exe".into(),
            icon: crate::gfx::ico::FOLDER,
            kind: CommandKind::Launch("explorer.exe".into()),
        }),
        "taskmgr" | "task manager" => Some(CommandItem {
            title: "Open Task Manager".into(),
            sub: "Launch taskmgr.exe".into(),
            icon: crate::gfx::ico::STATS,
            kind: CommandKind::Launch("taskmgr.exe".into()),
        }),
        "settings" | "prefs" => Some(CommandItem {
            title: "Nuva Settings".into(),
            sub: "Configure Nuva preferences".into(),
            icon: crate::gfx::ico::SETTINGS,
            kind: CommandKind::Action(crate::ui::Action::OpenSettings),
        }),
        "pick" | "color" | "eyedropper" => Some(CommandItem {
            title: "Screen Color Picker".into(),
            sub: "Pick any pixel on screen to copy HEX".into(),
            icon: crate::gfx::ico::EYEDROPPER,
            kind: CommandKind::Action(crate::ui::Action::PickColor),
        }),
        "mute" | "mic" => Some(CommandItem {
            title: "Toggle Microphone Mute".into(),
            sub: "Mute or unmute default microphone".into(),
            icon: crate::gfx::ico::MIC,
            kind: CommandKind::Action(crate::ui::Action::ToggleMicMute),
        }),
        "timer" | "pomo" => Some(CommandItem {
            title: "Focus Timer".into(),
            sub: "Switch to Pomodoro timer page".into(),
            icon: crate::gfx::ico::TIMER,
            kind: CommandKind::Page(1),
        }),
        "water" => Some(CommandItem {
            title: "Water Tracker".into(),
            sub: "Switch to hydration tracker page".into(),
            icon: crate::gfx::ico::DROP,
            kind: CommandKind::Page(2),
        }),
        "todo" | "tasks" => Some(CommandItem {
            title: "To-Do List".into(),
            sub: "Switch to tasks page".into(),
            icon: crate::gfx::ico::CHECKLIST,
            kind: CommandKind::Page(3),
        }),
        "calendar" | "cal" => Some(CommandItem {
            title: "Calendar".into(),
            sub: "Switch to calendar page".into(),
            icon: crate::gfx::ico::CALENDAR,
            kind: CommandKind::Page(4),
        }),
        "shelf" => Some(CommandItem {
            title: "File Shelf".into(),
            sub: "Switch to drag-and-drop shelf".into(),
            icon: crate::gfx::ico::FOLDER,
            kind: CommandKind::Page(5),
        }),
        "clipboard" | "clip" => Some(CommandItem {
            title: "Clipboard History".into(),
            sub: "Switch to clipboard history".into(),
            icon: crate::gfx::ico::CLIPBOARD,
            kind: CommandKind::Page(6),
        }),
        "notifs" | "notifications" => Some(CommandItem {
            title: "Notifications".into(),
            sub: "Switch to notification center".into(),
            icon: crate::gfx::ico::BELL,
            kind: CommandKind::Page(7),
        }),
        "stats" => Some(CommandItem {
            title: "System Stats".into(),
            sub: "Switch to hardware performance graphs".into(),
            icon: crate::gfx::ico::STATS,
            kind: CommandKind::Page(8),
        }),
        _ => None,
    }
}
