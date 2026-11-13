//! ssboot-pair's screen, for rootshell's terminal (Ghostty).
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
//!
//! What it shows is also kept as plain lines, the run's log, which it offers
//! to copy when it stops: through OSC 52, which Ghostty puts on the
//! clipboard (a WASM program can't run a command to do it).

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

/// The run's log: each line shown, plainly. Without times: reading the clock
/// would add a WASI import 0.3.0's module doesn't have.
struct Record {
    lines: Vec<String>,
    last: String,
}

static RECORD: Mutex<Record> = Mutex::new(Record { lines: Vec::new(), last: String::new() });

/// Keep `text` in the run's log, unless it repeats the line before.
fn record(text: &str) {
    let mut r = RECORD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() || line == r.last {
            continue;
        }
        r.last = line.to_string();
        // A run is a few dozen lines: a long one keeps its start and its end.
        if r.lines.len() >= 2000 {
            r.lines.remove(100);
        }
        r.lines.push(line.to_string());
    }
}

/// The run's log so far.
pub fn recorded() -> Vec<String> {
    RECORD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).lines.clone()
}

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

    /// Plain output's, as ssboot-pair always printed them.
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
    record(&format!("ssboot-pair {version} log"));
    let mut s = screen();
    let lines = if s.fancy {
        let mut lines = vec![
            String::new(),
            format!(
                "  {} {}",
                s.paint(BADGE, " ssboot-pair "),
                s.paint(DIM, version)
            ),
        ];
        lines.extend(s.hanging("  ", 2, "", "Pairs this iPhone with itself, for ssboot."));
        lines
    } else {
        vec![format!(
            "ssboot-pair {version} — pairs this iPhone with itself"
        )]
    };
    s.emit(&lines);
}

/// A line to keep in mind, marked.
pub fn say(mark: Mark, text: &str) {
    record(&format!("{} {text}", mark.word()));
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
    record(&format!("   {text}"));
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
    record(&format!("   {n}. {text}"));
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
    record(&format!("     {text}"));
    let mut s = screen();
    let line = format!("    {}", s.paint(ACCENT, text));
    s.emit(&[line]);
}

pub fn blank() {
    screen().emit(&[String::new()]);
}

/// The live line: what ssboot-pair is waiting for now.
pub fn live(text: &str) {
    record(&format!(" … {text}"));
    screen().set_live("◆", STRONG, text);
}

/// The live line with a spinner, for `time`: it moves only while the program
/// is awake, as it is between announcements.
pub fn spin(text: &str, time: Duration) {
    record(&format!(" … {text}"));
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
    record(&format!("Stopped: {problem}"));
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
    // Not the code itself: the log is meant to be pasted somewhere.
    record("     (a 6-digit code, in large digits)");
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

/// A question answered with a line: a WASM program can't read keys one at a
/// time. None at the end of input. `hint` follows it, dimmed: "y/n". The
/// question, hint and answer fit 40 columns, or the answer would wrap.
pub fn ask(question: &str, hint: &str) -> Option<String> {
    let s = screen();
    let prompt = if s.fancy {
        format!("  {} {} {} ", s.paint(STRONG, "◆"), s.paint(BOLD, question), s.paint(DIM, hint))
    } else {
        format!("{question} [{hint}] ")
    };
    drop(s);
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => {
            println!();
            None
        }
        Ok(_) => {
            let answer = line.trim().to_string();
            record(&format!(" ?  {question} {answer}"));
            Some(answer)
        }
    }
}

/// `text` onto the clipboard: OSC 52, which Ghostty, rootshell's terminal,
/// takes as a copy. Nothing says whether it did.
pub fn copy(text: &str) {
    if cfg!(test) {
        return;
    }
    print!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = std::io::stdout().flush();
}

/// Standard base64, with padding, as OSC 52 takes it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
    fn base64_is_the_standard_one_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64("Settings › Pair".as_bytes()), "U2V0dGluZ3Mg4oC6IFBhaXI=");
    }

    #[test]
    fn the_log_keeps_lines_not_codes() {
        // Other tests log too, into the same record: these lines are its own.
        header("9.9.9");
        live("Waiting for the log test…");
        live("Waiting for the log test…");
        big("975310");
        stopped("the log test ended.");
        let log = recorded().join("\n");
        assert!(log.contains("ssboot-pair 9.9.9 log"), "{log}");
        assert_eq!(log.matches("Waiting for the log test").count(), 1, "{log}");
        assert!(!log.contains("975310") && !log.contains("975 310"), "{log}");
        assert!(log.contains("Stopped: the log test ended."), "{log}");
    }

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
