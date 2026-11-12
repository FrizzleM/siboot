//! siboot-pair's screen, for rootshell's terminal (Ghostty).
//!
//! A WASM program there can't ask the terminal its size, runs on one thread
//! and blocks in every socket call. So the screen is made of what holds up
//! under those limits: lines that stay, and one live line at the bottom,
//! rewritten in place and kept short enough never to wrap. It moves while the
//! program is awake (the announcement burst); while it waits in `accept`, the
//! live line says what for. Codes are shown in large digits and posted as
//! notifications (OSC 777), which rootshell shows in the background too.
//!
//! Plain lines with `--plain` or TERM=dumb, and in the tests.

use std::io::Write as _;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

// SGR styles, in ANSI's own colors so the terminal's theme picks the shades.
const ACCENT: &str = "36";
const STRONG: &str = "1;36";
const GOOD: &str = "1;32";
const WARN: &str = "1;33";
const BAD: &str = "1;31";
const DIM: &str = "2";
const BOLD: &str = "1";
const BADGE: &str = "1;30;46";

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const FRAME: Duration = Duration::from_millis(80);
/// An iPhone's terminal upright is about 40 columns wide; COLUMNS, when
/// rootshell sets it, says better.
const DEFAULT_COLUMNS: usize = 40;

struct Screen {
    fancy: bool,
    color: bool,
    /// Columns a line may take.
    measure: usize,
    /// The live line on screen, if any: mark, its style, text.
    live: Option<(&'static str, &'static str, String)>,
    frame: usize,
    /// Plain output: the last live text printed, so a repeat isn't.
    last: String,
}

static SCREEN: Mutex<Screen> = Mutex::new(Screen {
    fancy: false,
    color: false,
    measure: DEFAULT_COLUMNS - 1,
    live: None,
    frame: 0,
    last: String::new(),
});

fn screen() -> MutexGuard<'static, Screen> {
    SCREEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn init(plain: bool) {
    let mut s = screen();
    s.fancy = !plain && !cfg!(test) && !matches!(std::env::var("TERM").as_deref(), Ok("dumb"));
    s.color = s.fancy && std::env::var_os("NO_COLOR").is_none();
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.trim().parse::<usize>().ok())
        .filter(|c| (20..=400).contains(c))
        .unwrap_or(DEFAULT_COLUMNS);
    s.measure = columns.min(72) - 1;
}

/// How a line that stays starts.
#[derive(Clone, Copy)]
pub enum Mark {
    Good,
    Warn,
    Info,
}

impl Mark {
    fn glyph(self) -> (&'static str, &'static str) {
        match self {
            Mark::Good => ("✓", GOOD),
            Mark::Warn => ("▲", WARN),
            Mark::Info => ("·", DIM),
        }
    }

    /// Plain output's, as siboot-pair always printed them.
    fn word(self) -> &'static str {
        match self {
            Mark::Good => "OK",
            Mark::Warn => "!!",
            Mark::Info => "  ",
        }
    }
}

impl Screen {
    fn paint(&self, style: &str, text: &str) -> String {
        if self.color && !style.is_empty() && !text.is_empty() {
            format!("\x1b[{style}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    fn live_line(&self) -> String {
        match &self.live {
            Some((mark, style, text)) => {
                let room = self.measure.saturating_sub(4);
                format!("  {} {}", self.paint(style, mark), cut(text, room))
            }
            None => String::new(),
        }
    }

    /// Print `lines` to stay, above the live line.
    fn emit(&mut self, lines: &[String]) {
        let mut out = String::new();
        if self.fancy && self.live.is_some() {
            out.push_str("\r\x1b[2K");
        }
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
        if self.fancy && self.live.is_some() {
            out.push_str(&self.live_line());
        }
        print!("{out}");
        let _ = std::io::stdout().flush();
    }

    fn set_live(&mut self, mark: &'static str, style: &'static str, text: &str) {
        if !self.fancy {
            if self.last != text {
                self.last = text.to_string();
                println!("  {text}");
            }
            return;
        }
        self.live = Some((mark, style, text.to_string()));
        print!("\r\x1b[2K{}", self.live_line());
        let _ = std::io::stdout().flush();
    }

    /// `text` after `lead`, wrapped, its other lines indented to match.
    fn hanging(&self, lead: &str, lead_width: usize, style: &str, text: &str) -> Vec<String> {
        let room = self.measure.saturating_sub(lead_width).max(10);
        wrap(text, room)
            .into_iter()
            .enumerate()
            .map(|(n, piece)| {
                let start = if n == 0 {
                    lead.to_string()
                } else {
                    " ".repeat(lead_width)
                };
                format!("{start}{}", self.paint(style, &piece))
            })
            .collect()
    }
}

/// The name and version, and what it does.
pub fn header(version: &str) {
    let mut s = screen();
    let lines = if s.fancy {
        let mut lines = vec![
            String::new(),
            format!(
                "  {} {}",
                s.paint(BADGE, " siboot-pair "),
                s.paint(DIM, version)
            ),
        ];
        lines.extend(s.hanging("  ", 2, "", "Pairs this iPhone with itself, for siboot."));
        lines
    } else {
        vec![format!(
            "siboot-pair {version} — pairs this iPhone with itself"
        )]
    };
    s.emit(&lines);
}

/// A line to keep in mind, marked.
pub fn say(mark: Mark, text: &str) {
    let mut s = screen();
    let lines = if s.fancy {
        let (glyph, style) = mark.glyph();
        let lead = format!("  {} ", s.paint(style, glyph));
        s.hanging(&lead, 4, "", text)
    } else {
        vec![format!("{} {text}", mark.word())]
    };
    s.emit(&lines);
}

/// A paragraph, wrapped; `style` is one of `heading`'s or `dim`'s.
fn paragraph(style: &str, text: &str) {
    let mut s = screen();
    let lines = if s.fancy {
        s.hanging("  ", 2, style, text)
    } else {
        vec![text.to_string()]
    };
    s.emit(&lines);
}

pub fn text(text: &str) {
    paragraph("", text);
}

pub fn heading(text: &str) {
    paragraph(BOLD, text);
}

pub fn dim(text: &str) {
    paragraph(DIM, text);
}

/// Step `n` of a list, its other lines indented under its text.
pub fn numbered(n: usize, text: &str) {
    let mut s = screen();
    let lines = if s.fancy {
        let lead = format!("  {} ", s.paint(STRONG, &format!("{n:>2}")));
        s.hanging(&lead, 5, "", text)
    } else {
        vec![format!("  {n}. {text}")]
    };
    s.emit(&lines);
}

/// A command to type, on one line however long, so it copies whole.
pub fn command(text: &str) {
    let mut s = screen();
    let line = format!("    {}", s.paint(ACCENT, text));
    s.emit(&[line]);
}

pub fn blank() {
    screen().emit(&[String::new()]);
}

/// The live line: what siboot-pair is waiting for now.
pub fn live(text: &str) {
    screen().set_live("◆", STRONG, text);
}

/// The live line with a spinner, for `time`: it moves only while the program
/// is awake, as it is between announcements.
pub fn spin(text: &str, time: Duration) {
    let frames = (time.as_millis() / FRAME.as_millis()).max(1);
    for _ in 0..frames {
        {
            let mut s = screen();
            if s.fancy {
                s.frame = s.frame.wrapping_add(1);
                let glyph = SPINNER[s.frame % SPINNER.len()];
                s.set_live(glyph, ACCENT, text);
            } else {
                s.set_live("", "", text);
            }
        }
        std::thread::sleep(FRAME);
    }
}

/// The live line ends as a line that stays.
pub fn settle(mark: Mark, text: &str) {
    let mut s = screen();
    if s.fancy && s.live.take().is_some() {
        print!("\r\x1b[2K");
    }
    s.last.clear();
    drop(s);
    say(mark, text);
}

/// The live line, if any, is left as it was, and the output ends with a newline.
pub fn end() {
    let mut s = screen();
    if s.fancy && s.live.take().is_some() {
        println!();
        let _ = std::io::stdout().flush();
    }
}

/// A failure: the live line, if any, becomes it.
pub fn stopped(problem: &str) {
    let mut s = screen();
    if s.fancy && s.live.take().is_some() {
        print!("\r\x1b[2K");
    }
    let lines = if s.fancy {
        let lead = format!("  {} ", s.paint(BAD, "✗"));
        let mut lines = vec![String::new()];
        lines.extend(s.hanging(&lead, 4, "", &format!("Stopped: {problem}")));
        lines
    } else {
        vec![String::new(), format!("Stopped: {problem}")]
    };
    s.emit(&lines);
}

/// `code` in large digits, three groups of three apart, and plainly under
/// them.
pub fn big(code: &str) {
    let mut s = screen();
    let spaced = spaced(code);
    if !s.fancy {
        s.emit(&[format!("    {spaced}")]);
        return;
    }
    let mut lines: Vec<String> = big_digits(code)
        .into_iter()
        .map(|row| format!("    {}", s.paint(STRONG, &row)))
        .collect();
    lines.push(format!("    {}", s.paint(DIM, &spaced)));
    s.emit(&lines);
}

/// A notification, which rootshell's terminal posts even with rootshell in
/// the background (OSC 777, as Ghostty reads it).
pub fn notify(title: &str, body: &str) {
    if cfg!(test) {
        return;
    }
    let clean = |t: &str| t.replace([';', '\x07', '\x1b'], " ");
    print!("\x1b]777;notify;{};{}\x07", clean(title), clean(body));
    let _ = std::io::stdout().flush();
}

/// "481 516".
pub fn spaced(code: &str) -> String {
    let (a, b) = code.split_at(code.len().min(3));
    format!("{a} {b}")
}

/// Digits three rows tall in heavy box-drawing lines, which Ghostty draws
/// itself, so they join up whatever the font.
const DIGITS: [[&str; 3]; 10] = [
    ["┏━┓", "┃ ┃", "┗━┛"],
    ["╺┓ ", " ┃ ", "╺┻╸"],
    ["╺━┓", "┏━┛", "┗━╸"],
    ["╺━┓", "╺━┫", "╺━┛"],
    ["╻ ╻", "┗━┫", "  ╹"],
    ["┏━╸", "┗━┓", "╺━┛"],
    ["┏━╸", "┣━┓", "┗━┛"],
    ["╺━┓", "  ┃", "  ╹"],
    ["┏━┓", "┣━┫", "┗━┛"],
    ["┏━┓", "┗━┫", "╺━┛"],
];

/// `code`'s three rows in large digits, a gap after the third.
fn big_digits(code: &str) -> [String; 3] {
    let mut rows: [String; 3] = Default::default();
    for (i, d) in code.chars().filter_map(|c| c.to_digit(10)).enumerate() {
        for (row, out) in rows.iter_mut().enumerate() {
            if i > 0 {
                out.push_str(if i % 3 == 0 { "   " } else { " " });
            }
            out.push_str(DIGITS[d as usize][row]);
        }
    }
    rows
}

/// Columns a character takes: two for East Asian wide characters and emoji.
fn char_width(c: char) -> usize {
    let cp = c as u32;
    if cp < 0x20 || (0x0300..=0x036f).contains(&cp) || (0xfe00..=0xfe0f).contains(&cp) {
        return 0;
    }
    let wide = (0x1100..=0x115f).contains(&cp)
        || (0x2e80..=0xa4cf).contains(&cp)
        || (0xac00..=0xd7a3).contains(&cp)
        || (0xf900..=0xfaff).contains(&cp)
        || (0xff00..=0xff60).contains(&cp)
        || (0x1f300..=0x1f6ff).contains(&cp)
        || (0x1f900..=0x1f9ff).contains(&cp);
    if wide {
        2
    } else {
        1
    }
}

fn width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// `text` cut to `room` columns, with … where it's cut.
fn cut(text: &str, room: usize) -> String {
    if width(text) <= room {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 1;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > room {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Words in lines of at most `room` columns; a longer word is cut.
fn wrap(text: &str, room: usize) -> Vec<String> {
    let room = room.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        let w = width(word);
        if used > 0 && used + 1 + w > room {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if w > room {
            for c in word.chars() {
                if used + char_width(c) > room {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(c);
                used += char_width(c);
            }
            continue;
        }
        if used > 0 {
            line.push(' ');
            used += 1;
        }
        line.push_str(word);
        used += w;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_digits_are_three_rows_of_even_width() {
        let rows = big_digits("481516");
        assert_eq!(rows[0], "╻ ╻ ┏━┓ ╺┓    ┏━╸ ╺┓  ┏━╸");
        assert_eq!(rows[1], "┗━┫ ┣━┫  ┃    ┗━┓  ┃  ┣━┓");
        assert_eq!(rows[2], "  ╹ ┗━┛ ╺┻╸   ╺━┛ ╺┻╸ ┗━┛");
        // Every digit is three columns, so the rows line up.
        for (d, glyph) in DIGITS.iter().enumerate() {
            assert!(glyph.iter().all(|row| width(row) == 3), "{d}");
        }
        assert!(rows.iter().all(|row| width(row) == 25));
    }

    #[test]
    fn codes_read_in_two_groups() {
        assert_eq!(spaced("481516"), "481 516");
    }

    #[test]
    fn lines_wrap_and_cut_to_the_screen() {
        assert_eq!(
            wrap("Open Settings › Privacy & Security › Developer Mode", 20),
            ["Open Settings ›", "Privacy & Security ›", "Developer Mode"]
        );
        assert_eq!(cut("Waiting for Settings to connect", 12), "Waiting for…");
        assert_eq!(cut("short", 12), "short");
    }
}
