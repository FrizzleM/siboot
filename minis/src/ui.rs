//! The terminal UI: a live screen on a terminal, plain lines everywhere else.
//!
//! On Minis's terminal, ssboot keeps a small live area at the bottom of what
//! it has printed: the running step with a spinner and what it's doing, a
//! progress bar, the steps still to come, and prompts that read keys (y/n,
//! menus, code boxes, a masked password). A finished step, a warning or an
//! answer is printed above that area and stays, so the scrollback reads as a
//! log of the run.
//!
//! The live area ends in the credit, dim, from the header on: it's under every
//! step and prompt, and still the last line when ssboot exits. It's the first
//! thing to go on a screen too short for it.
//!
//! The live area is redrawn in place, so it never wraps and never outgrows
//! the screen: each of its lines is cut to the terminal's width, and the area
//! to its height. Each step still says what it's doing before the work
//! starts, so a stall is attributable to a named step, and errors name the
//! thing the user can change while the raw cause goes to the verbose channel.
//!
//! Without a terminal on both ends, with `--plain` or `--verbose`, with
//! TERM=dumb or SSBOOT_PLAIN set, and in the tests, the output is plain lines,
//! one per event.

use std::fmt::Write as _;
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

// SGR styles, in ANSI's own colors so the terminal's theme picks the shades
// for a light or a dark background.
const ACCENT: &str = "36";
const STRONG: &str = "1;36";
const GOOD: &str = "1;32";
const WARN: &str = "1;33";
const BAD: &str = "1;31";
const DIM: &str = "2";
const BOLD: &str = "1";
const BADGE: &str = "1;30;46";
const NONE: &str = "";

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// How often the spinner moves and the live area is redrawn.
const TICK: Duration = Duration::from_millis(80);
/// The step names' column, after "  ✓ ".
const NAME_WIDTH: usize = 10;
/// Where a step's text, and an answer in a prompt, start.
const TEXT_COLUMN: usize = 4 + NAME_WIDTH;
/// A box is no wider than this, however wide the terminal is.
const BOX_MAX: usize = 60;
/// Plain output's indent under a `[n/total]` line.
const INDENT: &str = "      ";
/// Under the live area for as long as it's there, and after the version in
/// plain output.
const CREDIT: &str = "A project by Frizzle";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// One line per event, as a log.
    Plain,
    /// The live screen.
    Live,
    /// The live screen is over (done, stopped, or given up on): what's left
    /// prints as plain lines.
    Closed,
}

struct Ui {
    mode: Mode,
    color: bool,
    verbose: bool,
    /// Plain output numbers steps `[n/total]`.
    total: usize,
    /// The steps this run goes through.
    plan: Vec<&'static str>,
    /// Index into `plan` of the running step.
    active: Option<usize>,
    /// What the running step is doing now.
    status: String,
    step_started: Instant,
    run_started: Instant,
    bar: Option<Bar>,
    dialog: Option<Dialog>,
    /// What the user gave in this step so far, shown above its next prompt.
    answers: Vec<(String, String)>,
    /// The live area's rows, and which of them the cursor is on.
    rows: usize,
    cursor_row: usize,
    /// The last line printed above the live area is blank, so the credit,
    /// alone in the area, needs no blank line of its own.
    after_blank: bool,
    frame: usize,
    /// Plain output: the progress line being rewritten with `\r`, if any, and
    /// the last percentage printed for `plain_label`.
    plain_open: bool,
    plain_label: String,
    plain_percent: Option<u64>,
    /// The stop box is on screen (the run failed): Ctrl-C at the question
    /// after it only leaves.
    stopped: bool,
}

struct Bar {
    done: u64,
    total: u64,
    /// Bytes, shown as MB with the time left; else a percentage alone.
    bytes: bool,
    started: Instant,
}

/// A prompt's lines under the running step, and where its cursor goes.
struct Dialog {
    lines: Vec<Line>,
    cursor: Option<(usize, usize)>,
}

static UI: OnceLock<Mutex<Ui>> = OnceLock::new();

fn ui() -> MutexGuard<'static, Ui> {
    UI.get_or_init(|| Mutex::new(Ui::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn flush() {
    let _ = io::stdout().flush();
}

// ---------------------------------------------------------------- the log

/// Everything the run showed, as plain lines stamped with the time since it
/// started, and the libraries' own log lines: the log ssboot offers to copy
/// when it stops (`report.rs`).
struct Record {
    lines: Vec<String>,
    /// The last line kept, so a status shown again isn't.
    last: String,
    /// Lines let go from the middle, past `RECORD_MAX`.
    dropped: usize,
}

static RECORD: Mutex<Record> = Mutex::new(Record {
    lines: Vec::new(),
    last: String::new(),
    dropped: 0,
});
static STARTED: OnceLock<Instant> = OnceLock::new();
/// Lines kept: past it, the oldest after `RECORD_HEAD` go, so the log keeps
/// how the run started and how it ended.
const RECORD_MAX: usize = 4000;
const RECORD_HEAD: usize = 200;

fn record_lock() -> MutexGuard<'static, Record> {
    RECORD.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Keep `text` in the run's log, one entry per line of it.
pub fn record(text: &str) {
    let secs = STARTED.get_or_init(Instant::now).elapsed().as_secs();
    let stamp = format!("{:02}:{:02}", secs / 60, secs % 60);
    let mut r = record_lock();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() || line == r.last {
            continue;
        }
        r.last = line.to_string();
        if r.lines.len() >= RECORD_MAX {
            r.lines.remove(RECORD_HEAD);
            r.dropped += 1;
        }
        r.lines.push(format!("{stamp} {line}"));
    }
}

/// The run's log so far.
pub fn recorded() -> Vec<String> {
    let r = record_lock();
    let mut lines = r.lines.clone();
    if r.dropped > 0 {
        let at = RECORD_HEAD.min(lines.len());
        lines.insert(at, format!("      … {} lines left out …", r.dropped));
    }
    lines
}

/// tracing's output into the run's log, whatever the screen shows of it.
pub struct RecordLines;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RecordLines {
    type Writer = RecordLine;

    fn make_writer(&'a self) -> RecordLine {
        RecordLine(Vec::new())
    }
}

pub struct RecordLine(Vec<u8>);

impl Write for RecordLine {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RecordLine {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0);
        for line in text.lines() {
            record(&format!("   log {}", line.trim()));
        }
    }
}

/// Chooses the live screen or plain lines. Called once, before any output.
pub fn init(verbose: bool, plain: bool) {
    STARTED.get_or_init(Instant::now);
    let tty = io::stdout().is_terminal();
    let live = tty
        && io::stdin().is_terminal()
        && !verbose
        && !plain
        && !cfg!(test)
        && !matches!(std::env::var("TERM").as_deref(), Ok("dumb"))
        && std::env::var_os("SSBOOT_PLAIN").is_none()
        && term::start();
    let mut ui = ui();
    ui.verbose = verbose;
    ui.color = tty && std::env::var_os("NO_COLOR").is_none();
    if live {
        ui.mode = Mode::Live;
        std::thread::spawn(tick);
    }
}

pub fn is_live() -> bool {
    ui().mode == Mode::Live
}

/// Moves the spinner, the timer and the progress bar.
fn tick() {
    loop {
        std::thread::sleep(TICK);
        let mut ui = ui();
        match ui.mode {
            Mode::Live if ui.active.is_some() && ui.dialog.is_none() => {
                ui.frame = ui.frame.wrapping_add(1);
                ui.paint(&[]);
            }
            Mode::Live => {}
            _ => return,
        }
    }
}

/// The name, version and what this run does, with the one thing to keep in
/// mind while it runs.
pub fn header(version: &str, tagline: &str) {
    record(&format!("ssboot {version} — {tagline}"));
    let mut ui = ui();
    let note = "Keep Minis open until ssboot is done: iOS pauses it in the background.";
    if ui.mode != Mode::Live {
        println!();
        println!(
            "{} {}",
            ui.style(STRONG, "ssboot"),
            ui.style(DIM, &format!("{version} · {CREDIT}"))
        );
        println!("{}", ui.style(DIM, tagline));
        println!();
        println!("{}", ui.style(WARN, note));
        flush();
        return;
    }
    let max = line_width();
    let mut above = vec![String::new()];
    above.push(
        Line::new()
            .add(NONE, "  ")
            .add(BADGE, " ssboot ")
            .add(DIM, format!(" {version}"))
            .render(max, ui.color),
    );
    for piece in wrap(tagline, max.saturating_sub(2)) {
        above.push(
            Line::new()
                .add(NONE, "  ")
                .add(NONE, piece)
                .render(max, ui.color),
        );
    }
    above.extend(hanging(
        &Line::new().add(NONE, "  ").add(WARN, "▲ "),
        NONE,
        note,
        max,
        ui.color,
    ));
    above.push(String::new());
    ui.paint(&above);
}

/// The steps this run goes through, by name; plain output counts them out of
/// `total`.
pub fn plan(names: &[&'static str], total: usize) {
    let mut ui = ui();
    ui.plan = names.to_vec();
    ui.total = total;
}

/// Step `n` (from 1) starts: `title` says what it does.
pub fn step(n: usize, title: &str) {
    let mut ui = ui();
    record(&format!("[{n}/{}] {title}", ui.total));
    if ui.mode != Mode::Live {
        ui.end_plain_line();
        println!();
        println!(
            "{} {title}",
            ui.style(STRONG, &format!("[{n}/{}]", ui.total))
        );
        flush();
        return;
    }
    // A step still running when the next starts has finished.
    let status = ui.status.clone();
    let above = ui.settle("✓", GOOD, &status);
    ui.active = Some(n.saturating_sub(1));
    ui.status = lowercase_first(title);
    ui.step_started = Instant::now();
    ui.paint(&above);
}

/// The running step is done: `summary` stays on its line.
pub fn done(summary: &str) {
    record(&format!("    ✓ {summary}"));
    let mut ui = ui();
    if ui.mode == Mode::Live {
        let above = ui.settle("✓", GOOD, summary);
        ui.paint(&above);
    }
}

/// What the running step is doing now. Plain output prints it.
pub fn status(msg: &str) {
    record(&format!("      {msg}"));
    let mut ui = ui();
    match ui.mode {
        Mode::Live if ui.active.is_some() => {
            ui.status = msg.to_string();
            ui.paint(&[]);
        }
        Mode::Live => {
            let line = ui.note_lines(NONE, msg);
            ui.paint(&line);
        }
        _ => ui.plain(&format!("{INDENT}{msg}")),
    }
}

/// Like `status`, but plain output prints it only with `--verbose`: the live
/// screen shows each small step, a log only the ones that matter.
pub fn activity(msg: &str) {
    if is_live() {
        status(msg);
    } else {
        detail(msg);
    }
}

/// Something that worked.
pub fn ok(msg: &str) {
    record(&format!("   OK {msg}"));
    let mut ui = ui();
    match ui.mode {
        Mode::Live if ui.active.is_some() => {
            ui.status = msg.to_string();
            ui.paint(&[]);
        }
        Mode::Live => {
            let max = line_width();
            let lines = hanging(
                &Line::new().add(NONE, "  ").add(GOOD, "✓ "),
                NONE,
                msg,
                max,
                ui.color,
            );
            ui.paint(&lines);
        }
        _ => {
            let line = format!("{INDENT}{} {msg}", ui.style(GOOD, "OK"));
            ui.plain(&line);
        }
    }
}

/// Information that stays on screen.
pub fn note(msg: &str) {
    record(&format!("      {msg}"));
    let mut ui = ui();
    if ui.mode == Mode::Live {
        let lines = ui.note_lines(DIM, msg);
        ui.paint(&lines);
    } else {
        ui.plain(&format!("{INDENT}{msg}"));
    }
}

pub fn warn(msg: &str) {
    record(&format!("   !! {msg}"));
    let mut ui = ui();
    if ui.mode == Mode::Live {
        let max = line_width();
        let lines = hanging(
            &Line::new().add(NONE, "  ").add(WARN, "▲ "),
            NONE,
            msg,
            max,
            ui.color,
        );
        ui.paint(&lines);
    } else {
        let line = format!("{INDENT}{} {msg}", ui.style(WARN, "!!"));
        ui.plain(&line);
    }
}

/// Useful only when diagnosing. Shown with `--verbose`, which is plain.
pub fn detail(msg: &str) {
    record(&format!("   ·· {msg}"));
    let mut ui = ui();
    if ui.verbose && ui.mode != Mode::Live {
        let line = format!("{INDENT}{} {msg}", ui.style(DIM, "··"));
        ui.plain(&line);
    }
}

/// A library's log line (tracing), printed above the live area.
pub fn log(line: &str) {
    let mut ui = ui();
    if ui.mode == Mode::Live {
        let lines = ui.note_lines(DIM, line.trim());
        ui.paint(&lines);
    } else {
        eprintln!("{line}");
    }
}

/// Progress in bytes, under the running step.
pub fn progress(label: &str, done: u64, total: u64) {
    show_bar(label, done, total, true);
}

/// Progress as a percentage, under the running step.
pub fn percent(label: &str, percent: u64) {
    show_bar(label, percent.min(100), 100, false);
}

fn show_bar(label: &str, done: u64, total: u64, bytes: bool) {
    let mut ui = ui();
    if ui.mode != Mode::Live {
        let percent = (done.saturating_mul(100))
            .checked_div(total)
            .unwrap_or(100)
            .min(100);
        ui.plain_progress(label, percent);
        return;
    }
    match ui.bar.as_mut() {
        // The ticker redraws it.
        Some(bar) if bar.bytes == bytes => {
            bar.done = done;
            bar.total = total;
        }
        _ => {
            ui.bar = Some(Bar {
                done,
                total,
                bytes,
                started: Instant::now(),
            });
            ui.paint(&[]);
        }
    }
}

/// The progress bar is done with.
pub fn progress_end() {
    let mut ui = ui();
    if ui.mode == Mode::Live {
        ui.bar = None;
        ui.paint(&[]);
    } else {
        ui.end_plain_line();
        ui.plain_percent = None;
    }
}

/// The run worked: `title` and `paragraphs`, in a box.
pub fn finish(title: &str, paragraphs: &[&str]) {
    record(&format!("Done: {title}"));
    let mut ui = ui();
    if ui.mode != Mode::Live {
        ui.end_plain_line();
        println!();
        println!("{INDENT}{} {title}", ui.style(GOOD, "OK"));
        let room = if io::stdout().is_terminal() {
            term::size()
                .0
                .saturating_sub(INDENT.len() + 1)
                .clamp(20, 72)
        } else {
            72
        };
        for paragraph in paragraphs {
            for piece in wrap(paragraph, room) {
                println!("{INDENT}{piece}");
            }
        }
        flush();
        return;
    }
    let status = ui.status.clone();
    let mut above = ui.settle("✓", GOOD, &status);
    let took = clock(ui.run_started.elapsed().as_secs());
    above.push(String::new());
    above.extend(boxed(
        ui.color,
        GOOD,
        &format!("✓ {title}"),
        &took,
        paragraphs,
        term::size().0,
    ));
    above.push(String::new());
    ui.paint(&above);
    ui.close();
}

/// Report a failure: the advice always, the raw cause with `--verbose` (and
/// in the log). The live screen stays open for one more question: `close`
/// ends it.
pub fn failure(advice: &str, raw: &str) {
    record(&format!("Stopped: {advice}"));
    if !raw.is_empty() {
        record(&format!("   cause: {raw}"));
    }
    let mut ui = ui();
    if ui.mode != Mode::Live {
        ui.end_plain_line();
        flush();
        eprintln!();
        eprintln!("{} {advice}", ui.style(BAD, "Stopped:"));
        if ui.verbose {
            eprintln!("{INDENT}{} {raw}", ui.style(DIM, "cause:"));
        } else if !raw.is_empty() {
            eprintln!(
                "{INDENT}{}",
                ui.style(DIM, "run again with --verbose to see the underlying error")
            );
        }
        let _ = io::stderr().flush();
        return;
    }
    let above = ui.stopped(advice, !raw.is_empty());
    ui.paint(&above);
    ui.stopped = true;
}

/// Leave the terminal as it was. Safe to call more than once.
pub fn close() {
    ui().close();
}

// ---------------------------------------------------------------- prompts

/// A line of text, in a field labelled `label` showing `placeholder` while
/// it's empty, while the running step shows `question`. `plain` is the prompt
/// in plain output. None when the user stops (Esc, or the end of input).
pub fn ask_text(question: &str, label: &str, placeholder: &str, plain: &str) -> Option<String> {
    if !is_live() {
        let answer = ask(plain);
        record_answer(question, answer.as_ref().map(|_| "(typed)"));
        return answer;
    }
    let saved = swap_status(question);
    let asked = field(label, placeholder, false);
    swap_status(&saved);
    let answer = match asked {
        Asked::Answer(value) => value,
        Asked::NoKeys => ask(plain),
    };
    record_answer(question, answer.as_ref().map(|_| "(typed)"));
    answer
}

/// A prompt in the log: what it asked and, never what was typed, only
/// whether something was.
fn record_answer(question: &str, said: Option<&str>) {
    record(&format!("   ?  {} › {}", capitalized(question), said.unwrap_or("(stopped)")));
}

/// Like `ask_text`, shown as dots.
pub fn ask_secret(
    question: &str,
    label: &str,
    placeholder: &str,
    plain: &str,
) -> io::Result<Option<String>> {
    let answer = if is_live() {
        let saved = swap_status(question);
        let asked = field(label, placeholder, true);
        swap_status(&saved);
        match asked {
            Asked::Answer(value) => Ok(value),
            Asked::NoKeys => rpassword::prompt_password(format!("{INDENT}{plain}")).map(Some),
        }
    } else {
        rpassword::prompt_password(format!("{INDENT}{plain}")).map(Some)
    };
    let said = match &answer {
        Ok(Some(p)) if !p.is_empty() => Some("(hidden)"),
        _ => None,
    };
    record_answer(question, said);
    answer
}

/// A six-digit code, in boxes that fill as it's typed (the sixth digit sends
/// it), while the running step shows `question`; `hint` goes under the boxes.
pub fn ask_code(question: &str, label: &str, hint: &str, plain: &str) -> Option<String> {
    let answer = if is_live() {
        let saved = swap_status(question);
        let asked = code(label, &[Hint::Plain(hint.to_string())], false);
        swap_status(&saved);
        match asked {
            Asked::Answer(CodeAnswer::Code(digits)) => Some(digits),
            Asked::Answer(_) => None,
            Asked::NoKeys => ask(plain),
        }
    } else {
        ask(plain)
    };
    record_answer(question, answer.as_ref().map(|_| "(a code)"));
    answer
}

/// A line under the code boxes.
pub enum Hint {
    Plain(String),
    Warning(String),
}

pub enum CodeAnswer {
    Code(String),
    /// A letter was typed: the user wants another way to get the code.
    More,
    Cancel,
}

/// Live screen only: a six-digit code, while the running step shows
/// `question`; a letter asks for `More`. None on plain output, or when keys
/// can't be read one at a time after all (the screen is plain from then on):
/// then ask the plain way.
pub fn ask_code_or_more(question: &str, label: &str, hints: &[Hint]) -> Option<CodeAnswer> {
    if !is_live() {
        return None;
    }
    let saved = swap_status(question);
    let answer = match code(label, hints, true) {
        Asked::Answer(answer) => Some(answer),
        Asked::NoKeys => None,
    };
    swap_status(&saved);
    match &answer {
        Some(CodeAnswer::Code(_)) => record_answer(question, Some("(a code)")),
        Some(CodeAnswer::More) => record_answer(question, Some("other ways")),
        Some(CodeAnswer::Cancel) => record_answer(question, None),
        None => {}
    }
    answer
}

/// A yes or no: `y` and `n` answer at once, the arrows move, return takes
/// the highlighted one, which starts as `default`. With no step running (the
/// run has stopped), the question goes above the choice.
pub fn confirm(question: &str, default: bool) -> bool {
    let answer = confirm_prompt(question, default);
    record_answer(question, Some(if answer { "Yes" } else { "No" }));
    answer
}

fn confirm_prompt(question: &str, default: bool) -> bool {
    if !is_live() {
        let hint = if default { "[Y/n]" } else { "[y/N]" };
        return match ask(&format!("{} {hint} ", capitalized(question))) {
            Some(answer) if answer.is_empty() => default,
            Some(answer) => matches!(answer.to_lowercase().as_str(), "y" | "yes"),
            None => false,
        };
    }
    let saved = swap_status(question);
    let Some(mut keys) = term::Keys::open() else {
        degrade();
        swap_status(&saved);
        return confirm_prompt(question, default);
    };
    let mut yes = default;
    let answer = loop {
        {
            let mut ui = ui();
            let mut lines = Vec::new();
            let mut line = if ui.active.is_some() {
                rail().add(NONE, " ".repeat(NAME_WIDTH))
            } else {
                lines.push(
                    Line::new()
                        .add(NONE, "  ")
                        .add(STRONG, "◆ ")
                        .add(BOLD, capitalized(question)),
                );
                Line::new().add(NONE, "    ")
            };
            line = choice(line, "Yes", yes).add(NONE, "   ");
            line = choice(line, "No", !yes).add(DIM, "   y/n");
            lines.push(line);
            ui.show_dialog(lines, None);
        }
        match keys.next() {
            Key::Char('y' | 'Y') => break true,
            Key::Char('n' | 'N') => break false,
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Tab | Key::Char('h' | 'l') => {
                yes = !yes
            }
            Key::Enter => break yes,
            Key::Esc | Key::End => break false,
            Key::Interrupt => interrupt(),
            _ => {}
        }
    };
    drop(keys);
    swap_status(&saved);
    let mut ui = ui();
    let said = ui.note_lines(
        DIM,
        &format!(
            "{} › {}",
            capitalized(question),
            if answer { "Yes" } else { "No" }
        ),
    );
    ui.dialog = None;
    ui.paint(&said);
    answer
}

/// One of `options`, while the running step shows `title`: the arrows move,
/// a digit or return picks. None when the user backs out (Esc).
pub fn choose(title: &str, options: &[String]) -> Option<usize> {
    let picked = choose_prompt(title, options);
    record_answer(title, picked.and_then(|i| options.get(i)).map(String::as_str));
    picked
}

fn choose_prompt(title: &str, options: &[String]) -> Option<usize> {
    if options.is_empty() {
        return None;
    }
    if !is_live() {
        println!("{INDENT}{}", capitalized(title));
        for (i, option) in options.iter().enumerate() {
            println!("{INDENT}  {}: {option}", i + 1);
        }
        let line = ask("Number (empty to stop): ")?;
        return line
            .trim()
            .parse::<usize>()
            .ok()?
            .checked_sub(1)
            .filter(|i| *i < options.len());
    }
    let saved = swap_status(title);
    let Some(mut keys) = term::Keys::open() else {
        degrade();
        swap_status(&saved);
        return choose_prompt(title, options);
    };
    let mut at = 0;
    let picked = loop {
        {
            let mut ui = ui();
            let mut lines: Vec<Line> = options
                .iter()
                .enumerate()
                .map(|(i, option)| {
                    let line = rail().add(NONE, " ".repeat(NAME_WIDTH - 2));
                    if i == at {
                        line.add(STRONG, "› ").add(BOLD, option)
                    } else {
                        line.add(NONE, "  ").add(NONE, option)
                    }
                })
                .collect();
            let digits = if options.len() <= 9 {
                format!(" · 1–{}", options.len())
            } else {
                String::new()
            };
            lines.push(
                rail()
                    .add(NONE, " ".repeat(NAME_WIDTH))
                    .add(DIM, format!("↑↓ ↵{digits} · esc back")),
            );
            ui.show_dialog(lines, None);
        }
        match keys.next() {
            Key::Up | Key::Char('k') => at = (at + options.len() - 1) % options.len(),
            Key::Down | Key::Tab | Key::Char('j') => at = (at + 1) % options.len(),
            Key::Char(c) if c.is_ascii_digit() && options.len() <= 9 => {
                let n = c.to_digit(10).unwrap_or(0) as usize;
                if (1..=options.len()).contains(&n) {
                    break Some(n - 1);
                }
            }
            Key::Enter => break Some(at),
            Key::Esc | Key::End => break None,
            Key::Interrupt => interrupt(),
            _ => {}
        }
    };
    drop(keys);
    swap_status(&saved);
    let mut ui = ui();
    let said = match picked {
        Some(i) => ui.note_lines(DIM, &format!("{} › {}", capitalized(title), options[i])),
        None => Vec::new(),
    };
    ui.dialog = None;
    ui.paint(&said);
    picked
}

/// A line from the terminal, as plain output asks: None at the end of input.
pub fn ask(prompt: &str) -> Option<String> {
    print!("{INDENT}{prompt}");
    flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_string()),
    }
}

/// What a live prompt got: an answer, or no keys to read (then the caller
/// asks the plain way).
enum Asked<T> {
    Answer(T),
    NoKeys,
}

fn field(label: &str, placeholder: &str, secret: bool) -> Asked<Option<String>> {
    let Some(mut keys) = term::Keys::open() else {
        degrade();
        return Asked::NoKeys;
    };
    let mut value = String::new();
    let mut nudge = false;
    let given = loop {
        {
            let mut ui = ui();
            let room = line_width().saturating_sub(TEXT_COLUMN + 1).max(4);
            let shown = if secret {
                "•".repeat(value.chars().count())
            } else {
                value.clone()
            };
            let mut lines = ui.answer_lines();
            let mut line = rail().add(BOLD, pad(label, NAME_WIDTH));
            let col = if shown.is_empty() {
                if nudge {
                    line = line.add(WARN, "type it, or esc to stop");
                } else {
                    line = line.add(DIM, placeholder);
                }
                TEXT_COLUMN
            } else {
                let visible = keep_end(&shown, room);
                let col = TEXT_COLUMN + width(&visible);
                line = line.add(NONE, visible);
                col
            };
            lines.push(line);
            let row = lines.len() - 1;
            ui.show_dialog(lines, Some((row, col)));
        }
        match keys.next() {
            Key::Char(c) if !c.is_control() => {
                value.push(c);
                nudge = false;
            }
            Key::Backspace => {
                value.pop();
            }
            Key::ClearLine => value.clear(),
            Key::DeleteWord => {
                let kept = value.trim_end().rfind(' ').map_or(0, |i| i + 1);
                value.truncate(kept);
            }
            Key::Enter if !value.trim().is_empty() => break Some(value.trim().to_string()),
            Key::Enter => nudge = true,
            Key::Esc | Key::End => break None,
            Key::Interrupt => interrupt(),
            _ => {}
        }
    };
    drop(keys);
    let mut ui = ui();
    if let Some(value) = &given {
        let shown = if secret {
            "•".repeat(8)
        } else {
            value.clone()
        };
        ui.answers.push((label.to_string(), shown));
    }
    ui.dialog = None;
    ui.paint(&[]);
    Asked::Answer(given)
}

fn code(label: &str, hints: &[Hint], more: bool) -> Asked<CodeAnswer> {
    const DIGITS: usize = 6;
    let Some(mut keys) = term::Keys::open() else {
        degrade();
        return Asked::NoKeys;
    };
    let mut typed = String::new();
    let answer = loop {
        {
            let mut ui = ui();
            let mut lines = ui.answer_lines();
            let (boxes, col) = code_boxes(&typed, DIGITS);
            lines.push(rail().add(BOLD, pad(label, NAME_WIDTH)).join(boxes));
            let row = lines.len() - 1;
            let max = line_width();
            for hint in hints {
                let lead = rail().add(NONE, " ".repeat(NAME_WIDTH));
                lines.extend(match hint {
                    Hint::Plain(text) => wrapped(lead.clone(), Some(lead), DIM, text, None, max, 2),
                    Hint::Warning(text) => {
                        let more = lead.clone().add(NONE, "  ");
                        wrapped(lead.add(WARN, "▲ "), Some(more), NONE, text, None, max, 2)
                    }
                });
            }
            ui.show_dialog(lines, Some((row, TEXT_COLUMN + col)));
        }
        match keys.next() {
            Key::Char(c) if c.is_ascii_digit() => {
                typed.push(c);
                if typed.len() == DIGITS {
                    break CodeAnswer::Code(typed);
                }
            }
            // How codes are written out: "123 456", "123-456".
            Key::Char(' ' | '-') => {}
            Key::Backspace => {
                typed.pop();
            }
            Key::ClearLine => typed.clear(),
            Key::Char(_) | Key::Tab | Key::Down if more => break CodeAnswer::More,
            Key::Esc | Key::End => break CodeAnswer::Cancel,
            Key::Interrupt => interrupt(),
            _ => {}
        }
    };
    drop(keys);
    let mut ui = ui();
    ui.dialog = None;
    ui.paint(&[]);
    Asked::Answer(answer)
}

/// "4 8 1  _ _ _", and the column of the next box to fill.
fn code_boxes(typed: &str, digits: usize) -> (Line, usize) {
    let mut line = Line::new();
    let mut col = 0;
    let mut cursor = None;
    let mut typed = typed.chars();
    for i in 0..digits {
        if i > 0 {
            let gap = if i % 3 == 0 { "  " } else { " " };
            line = line.add(NONE, gap);
            col += gap.len();
        }
        match typed.next() {
            Some(digit) => line = line.add(STRONG, digit.to_string()),
            None => {
                cursor.get_or_insert(col);
                line = line.add(DIM, "_");
            }
        }
        col += 1;
    }
    (line, cursor.unwrap_or(col))
}

fn choice(line: Line, label: &str, on: bool) -> Line {
    if on {
        line.add(STRONG, "● ").add(BOLD, label)
    } else {
        line.add(DIM, "○ ").add(DIM, label)
    }
}

/// The running step shows `text` instead of its status; returns the status.
fn swap_status(text: &str) -> String {
    let mut ui = ui();
    std::mem::replace(&mut ui.status, text.to_string())
}

/// Ctrl-C at a prompt: stop now, as the signal would.
fn interrupt() -> ! {
    {
        let mut ui = ui();
        if ui.mode == Mode::Live {
            let above = if ui.stopped {
                ui.dialog = None;
                Vec::new()
            } else {
                ui.stopped("Stopped with Ctrl-C.", false)
            };
            ui.paint(&above);
        }
        ui.close();
    }
    term::restore();
    std::process::exit(130);
}

/// Keys can't be read one at a time here after all: finish with plain lines.
fn degrade() {
    let mut ui = ui();
    if ui.mode == Mode::Live {
        ui.dialog = None;
        ui.bar = None;
        let status = ui.status.clone();
        let above = ui.settle("·", DIM, &status);
        ui.paint(&above);
        ui.close();
    }
}

/// A panic's message goes under the live area, on a terminal left usable.
/// The panicking thread may hold the lock, so it isn't waited for.
fn on_panic() {
    if let Some(Ok(mut ui)) = UI.get().map(|m| m.try_lock()) {
        if ui.mode == Mode::Live {
            let below = ui.rows.saturating_sub(ui.cursor_row + 1);
            if below > 0 {
                term::write(&format!("\x1b[{below}B"));
            }
            ui.mode = Mode::Closed;
        }
    }
    term::restore();
    term::write("\r\n");
}

// ---------------------------------------------------------------- drawing

impl Ui {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            mode: Mode::Plain,
            color: false,
            verbose: false,
            total: 0,
            plan: Vec::new(),
            active: None,
            status: String::new(),
            step_started: now,
            run_started: now,
            bar: None,
            dialog: None,
            answers: Vec::new(),
            rows: 0,
            cursor_row: 0,
            after_blank: true,
            frame: 0,
            plain_open: false,
            plain_label: String::new(),
            plain_percent: None,
            stopped: false,
        }
    }

    fn style(&self, style: &str, text: &str) -> String {
        paint(self.color, style, text)
    }

    /// Redraw the live area, after printing `above` over it: lines that stay.
    fn paint(&mut self, above: &[String]) {
        if self.mode != Mode::Live {
            for line in above {
                println!("{line}");
            }
            flush();
            return;
        }
        if let Some(last) = above.last() {
            self.after_blank = last.is_empty();
        }
        let (cols, rows) = term::size();
        let (lines, cursor) = self.compose(cols, rows);
        // Synchronized, where the terminal supports it, so a frame never
        // shows half drawn.
        let mut out = String::from("\x1b[?2026h");
        if self.cursor_row > 0 {
            let _ = write!(out, "\x1b[{}A", self.cursor_row);
        }
        out.push('\r');
        for line in above {
            out.push_str(line);
            out.push_str("\x1b[K\r\n");
        }
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                out.push_str("\r\n");
            }
            out.push_str(line);
            out.push_str("\x1b[K");
        }
        out.push_str("\x1b[J");
        let last = lines.len().saturating_sub(1);
        match cursor {
            Some((row, col)) if row <= last => {
                if last > row {
                    let _ = write!(out, "\x1b[{}A", last - row);
                }
                out.push('\r');
                if col > 0 {
                    let _ = write!(out, "\x1b[{col}C");
                }
                out.push_str("\x1b[?25h");
                self.cursor_row = row;
            }
            _ => {
                out.push_str("\x1b[?25l");
                self.cursor_row = last;
            }
        }
        out.push_str("\x1b[?2026l");
        term::write(&out);
        self.rows = lines.len();
    }

    /// The live area: the running step, then its prompt, or its progress bar
    /// and the steps to come.
    fn compose(&self, cols: usize, rows: usize) -> (Vec<String>, Option<(usize, usize)>) {
        let max = cols.saturating_sub(1).max(20);
        let mut lines = Vec::new();
        let mut cursor = None;
        if let Some(i) = self.active {
            let (glyph, style) = if self.dialog.is_some() {
                ("◆", STRONG)
            } else {
                (SPINNER[self.frame % SPINNER.len()], ACCENT)
            };
            let name = self.plan.get(i).copied().unwrap_or("");
            let lead = Line::new()
                .add(NONE, "  ")
                .add(style, glyph)
                .add(NONE, " ")
                .add(BOLD, pad(name, NAME_WIDTH));
            let secs = self.step_started.elapsed().as_secs();
            let timer = (self.dialog.is_none() && secs >= 3).then(|| format!(" · {}", clock(secs)));
            lines.extend(wrapped(
                lead,
                None,
                NONE,
                &self.status,
                timer.as_deref(),
                max,
                2,
            ));
        }
        match &self.dialog {
            Some(dialog) => {
                let top = lines.len();
                lines.extend(dialog.lines.iter().cloned());
                cursor = dialog.cursor.map(|(row, col)| (top + row, col));
            }
            None => {
                if let Some(bar) = &self.bar {
                    lines.push(bar.line(max));
                }
                if let Some(i) = self.active {
                    for name in self.plan.iter().skip(i + 1) {
                        lines.push(Line::new().add(NONE, "  ").add(DIM, "○ ").add(DIM, *name));
                    }
                }
            }
        }
        // Never taller than the screen, or the next redraw couldn't reach its
        // top: the steps to come go first, and a prompt keeps its cursor.
        let room = rows.saturating_sub(1).max(1);
        if lines.len() > room {
            let start = match cursor {
                Some((row, _)) if row >= room => row + 1 - room,
                _ => 0,
            };
            lines = lines.into_iter().skip(start).take(room).collect();
            cursor = cursor.map(|(row, col)| (row - start, col));
        }
        // The credit, under a blank line, only where both fit: it never takes
        // a row from the run.
        let gap = !(lines.is_empty() && self.after_blank);
        if lines.len() + usize::from(gap) < room {
            if gap {
                lines.push(Line::new());
            }
            lines.push(Line::new().add(NONE, "  ").add(DIM, CREDIT));
        }
        (
            lines
                .iter()
                .map(|line| line.render(max, self.color))
                .collect(),
            cursor,
        )
    }

    /// The running step's last line, to print above the live area as it ends.
    fn settle(&mut self, mark: &str, style: &'static str, text: &str) -> Vec<String> {
        let Some(i) = self.active.take() else {
            return Vec::new();
        };
        self.bar = None;
        self.dialog = None;
        self.answers.clear();
        let name = self.plan.get(i).copied().unwrap_or("");
        let text_style = if style == BAD { BAD } else { DIM };
        let lead = Line::new()
            .add(NONE, "  ")
            .add(style, mark)
            .add(NONE, " ")
            .add(NONE, pad(name, NAME_WIDTH));
        hanging(&lead, text_style, text, line_width(), self.color)
    }

    /// The failed step and the advice, boxed.
    fn stopped(&mut self, advice: &str, more: bool) -> Vec<String> {
        let took = clock(self.step_started.elapsed().as_secs());
        let mut above = self.settle("✗", BAD, &format!("stopped after {took}"));
        let total = clock(self.run_started.elapsed().as_secs());
        above.push(String::new());
        above.extend(boxed(
            self.color,
            BAD,
            "✗ Stopped",
            &total,
            &[advice],
            term::size().0,
        ));
        if more {
            let hint = "Run ssboot again with --verbose to see the underlying error.";
            above.extend(hanging(
                &Line::new().add(NONE, "  "),
                DIM,
                hint,
                line_width(),
                self.color,
            ));
        }
        above.push(String::new());
        above
    }

    /// A line that stays, as `style`, under the step lines' text column.
    fn note_lines(&self, style: &'static str, text: &str) -> Vec<String> {
        hanging(
            &Line::new().add(NONE, "  ").add(DIM, "· "),
            style,
            text,
            line_width(),
            self.color,
        )
    }

    fn answer_lines(&self) -> Vec<Line> {
        self.answers
            .iter()
            .map(|(label, value)| rail().add(DIM, pad(label, NAME_WIDTH)).add(NONE, value))
            .collect()
    }

    fn show_dialog(&mut self, lines: Vec<Line>, cursor: Option<(usize, usize)>) {
        self.dialog = Some(Dialog { lines, cursor });
        self.paint(&[]);
    }

    fn close(&mut self) {
        if self.mode == Mode::Live {
            self.mode = Mode::Closed;
            // The live area stays on screen, credit and all: what comes next
            // (the shell's prompt) starts on a line under it.
            if self.rows > 0 {
                let below = self.rows.saturating_sub(self.cursor_row + 1);
                let down = if below > 0 {
                    format!("\x1b[{below}B")
                } else {
                    String::new()
                };
                term::write(&format!("{down}\r\n"));
            }
            term::restore();
        }
    }

    /// A plain line, after ending a progress line left open.
    fn plain(&mut self, line: &str) {
        self.end_plain_line();
        println!("{line}");
        flush();
    }

    fn end_plain_line(&mut self) {
        if self.plain_open {
            println!();
            self.plain_open = false;
        }
    }

    /// A percentage rewritten in place on a terminal (every 5 %), on lines of
    /// its own elsewhere (every 25 %).
    fn plain_progress(&mut self, label: &str, percent: u64) {
        if self.plain_label != label {
            self.end_plain_line();
            self.plain_label = label.to_string();
            self.plain_percent = None;
        }
        let tty = io::stdout().is_terminal();
        let every = if tty { 5 } else { 25 };
        if self.plain_percent == Some(percent) || (!percent.is_multiple_of(every) && percent != 100)
        {
            return;
        }
        self.plain_percent = Some(percent);
        if tty {
            print!("\r{INDENT}{label} {percent:>3}%");
            self.plain_open = true;
            if percent >= 100 {
                println!();
                self.plain_open = false;
            }
        } else {
            println!("{INDENT}{label} {percent}%");
        }
        flush();
    }
}

impl Bar {
    fn line(&self, max: usize) -> Line {
        let fraction = if self.total == 0 {
            1.0
        } else {
            (self.done as f64 / self.total as f64).clamp(0.0, 1.0)
        };
        let room = max.saturating_sub(TEXT_COLUMN);
        let mut tail = format!(" {:>3}%", (fraction * 100.0) as u64);
        if self.bytes {
            let size = format!("  {}/{} MB", megabytes(self.done), megabytes(self.total));
            if room >= 12 + width(&tail) + width(&size) {
                tail.push_str(&size);
                if let Some(left) = self.time_left() {
                    let left = format!(" · {left}");
                    if room >= 16 + width(&tail) + width(&left) {
                        tail.push_str(&left);
                    }
                }
            }
        }
        let cells = room.saturating_sub(width(&tail)).clamp(4, 32);
        let exact = fraction * cells as f64;
        let full = (exact.floor() as usize).min(cells);
        let half = full < cells && exact - full as f64 >= 0.5;
        let rest = cells - full - usize::from(half);
        let fill = if fraction >= 1.0 { GOOD } else { ACCENT };
        Line::new()
            .add(NONE, " ".repeat(TEXT_COLUMN))
            .add(fill, "━".repeat(full))
            .add(fill, if half { "╸" } else { "" })
            .add(DIM, "─".repeat(rest))
            .add(NONE, tail)
    }

    fn time_left(&self) -> Option<String> {
        let secs = self.started.elapsed().as_secs_f64();
        if secs < 2.0 || self.done == 0 || self.done >= self.total {
            return None;
        }
        let left = ((self.total - self.done) as f64 * secs / self.done as f64).ceil() as u64;
        Some(if left < 60 {
            format!("{left} s left")
        } else {
            format!("{} min left", left.div_ceil(60))
        })
    }
}

/// `text` after `lead` in at most `limit` lines, the others after `more`
/// (spaces, when None) and the last cut short if it must be; `tail` (dim)
/// goes after it when there's room.
fn wrapped(
    lead: Line,
    more: Option<Line>,
    style: &'static str,
    text: &str,
    tail: Option<&str>,
    max: usize,
    limit: usize,
) -> Vec<Line> {
    let indent = lead.width();
    let more = more.unwrap_or_else(|| Line::new().add(NONE, " ".repeat(indent)));
    let room = max.saturating_sub(indent).max(10);
    let mut pieces = wrap(text, room);
    if pieces.len() > limit {
        // Cut by `render`, with its …
        let rest = pieces.split_off(limit - 1).join(" ");
        pieces.push(rest);
    }
    let last = pieces.len() - 1;
    pieces
        .into_iter()
        .enumerate()
        .map(|(n, piece)| {
            let fits = tail.filter(|t| n == last && width(&piece) + width(t) <= room);
            let line = if n == 0 { lead.clone() } else { more.clone() };
            let line = line.add(style, piece);
            match fits {
                Some(tail) => line.add(DIM, tail),
                None => line,
            }
        })
        .collect()
}

/// The prompt's rail, under the running step's mark.
fn rail() -> Line {
    Line::new().add(NONE, "  ").add(ACCENT, "│ ")
}

/// The terminal's width for a line: one column short, so a full line never
/// leaves the cursor waiting to wrap.
fn line_width() -> usize {
    term::size().0.saturating_sub(1).max(20)
}

/// `text` wrapped after `lead`, its other lines indented to match.
fn hanging(lead: &Line, style: &'static str, text: &str, max: usize, color: bool) -> Vec<String> {
    let indent = lead.width();
    let room = max.saturating_sub(indent).max(10);
    wrap(text, room)
        .into_iter()
        .enumerate()
        .map(|(n, piece)| {
            let line = if n == 0 {
                lead.clone()
            } else {
                Line::new().add(NONE, " ".repeat(indent))
            };
            line.add(style, piece).render(max, color)
        })
        .collect()
}

/// `paragraphs` in a box `cols` wide at most, with `title` in its top edge and
/// `right` at the end of it.
fn boxed(
    color: bool,
    style: &'static str,
    title: &str,
    right: &str,
    paragraphs: &[&str],
    cols: usize,
) -> Vec<String> {
    let outer = cols.saturating_sub(3).clamp(24, BOX_MAX);
    let inner = outer - 4;
    let title = format!(" {title} ");
    let right = if right.is_empty() {
        String::new()
    } else {
        format!(" {right} ")
    };
    let fill = outer.saturating_sub(4 + width(&title) + width(&right));
    let mut out = vec![Line::new()
        .add(NONE, "  ")
        .add(style, "╭─")
        .add(style, title)
        .add(style, "─".repeat(fill))
        .add(DIM, right)
        .add(style, "─╮")
        .render(cols, color)];
    for (i, paragraph) in paragraphs.iter().enumerate() {
        let mut pieces = wrap(paragraph, inner);
        if i > 0 {
            pieces.insert(0, String::new());
        }
        for piece in pieces {
            let gap = inner.saturating_sub(width(&piece));
            out.push(
                Line::new()
                    .add(NONE, "  ")
                    .add(style, "│ ")
                    .add(NONE, piece)
                    .add(NONE, " ".repeat(gap))
                    .add(style, " │")
                    .render(cols, color),
            );
        }
    }
    out.push(
        Line::new()
            .add(NONE, "  ")
            .add(style, format!("╰{}╯", "─".repeat(outer - 2)))
            .render(cols, color),
    );
    out
}

// ---------------------------------------------------------------- text

/// Styled pieces of one line.
#[derive(Clone, Default)]
struct Line(Vec<(&'static str, String)>);

impl Line {
    fn new() -> Self {
        Self::default()
    }

    fn add(mut self, style: &'static str, text: impl Into<String>) -> Self {
        self.0.push((style, text.into()));
        self
    }

    fn join(mut self, other: Line) -> Self {
        self.0.extend(other.0);
        self
    }

    fn width(&self) -> usize {
        self.0.iter().map(|(_, text)| width(text)).sum()
    }

    /// The line for the terminal, cut to `max` columns, with … where it's cut.
    fn render(&self, max: usize, color: bool) -> String {
        let cut = self.width() > max;
        let budget = if cut { max.saturating_sub(1) } else { max };
        let mut used = 0;
        let mut out = String::new();
        for (style, text) in &self.0 {
            let mut piece = String::new();
            let mut whole = true;
            for c in text.chars() {
                let w = char_width(c);
                if used + w > budget {
                    whole = false;
                    break;
                }
                piece.push(c);
                used += w;
            }
            out.push_str(&paint(color, style, &piece));
            if !whole {
                break;
            }
        }
        if cut {
            out.push('…');
        }
        out
    }
}

fn paint(color: bool, style: &str, text: &str) -> String {
    if color && !style.is_empty() && !text.is_empty() {
        format!("\x1b[{style}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Columns a character takes in a terminal: none for controls and combining
/// marks, two for East Asian wide characters and emoji.
fn char_width(c: char) -> usize {
    let cp = c as u32;
    let zero = cp < 0x20
        || (0x7f..0xa0).contains(&cp)
        || (0x0300..=0x036f).contains(&cp)
        || (0x200b..=0x200f).contains(&cp)
        || (0xfe00..=0xfe0f).contains(&cp);
    if zero {
        return 0;
    }
    let wide = (0x1100..=0x115f).contains(&cp)
        || (0x2e80..=0xa4cf).contains(&cp)
        || (0xac00..=0xd7a3).contains(&cp)
        || (0xf900..=0xfaff).contains(&cp)
        || (0xfe30..=0xfe4f).contains(&cp)
        || (0xff00..=0xff60).contains(&cp)
        || (0xffe0..=0xffe6).contains(&cp)
        || (0x1f300..=0x1f64f).contains(&cp)
        || (0x1f680..=0x1f6ff).contains(&cp)
        || (0x1f900..=0x1f9ff).contains(&cp)
        || (0x20000..=0x3fffd).contains(&cp);
    if wide {
        2
    } else {
        1
    }
}

pub fn width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// `text`, then spaces to `columns`.
fn pad(text: &str, columns: usize) -> String {
    let gap = columns.saturating_sub(width(text));
    format!(
        "{text}{}",
        " ".repeat(gap.max(usize::from(width(text) >= columns)))
    )
}

/// Words in lines of at most `room` columns; a longer word (a URL) is cut.
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
                let cw = char_width(c);
                if used + cw > room {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(c);
                used += cw;
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

/// The end of `text` that fits in `room` columns, after … if cut.
fn keep_end(text: &str, room: usize) -> String {
    if width(text) <= room {
        return text.to_string();
    }
    let mut kept = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let w = char_width(c);
        if used + w > room {
            break;
        }
        kept.push(c);
        used += w;
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// 0:07, 3:41, 1:02:09.
fn clock(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_048_576.0)
}

// ---------------------------------------------------------------- keys

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Char(char),
    Enter,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Tab,
    Esc,
    /// Ctrl-C.
    Interrupt,
    /// Ctrl-D, or the end of input.
    End,
    /// Ctrl-U.
    ClearLine,
    /// Ctrl-W.
    DeleteWord,
    Ignored,
}

/// Keys in `bytes`, and how many bytes they took: a sequence cut short at the
/// end is left for the next read.
fn parse_keys(bytes: &[u8]) -> (Vec<Key>, usize) {
    let mut keys = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let (key, len) = match bytes[i] {
            0x1b => match bytes.get(i + 1) {
                // CSI or SS3: parameters, then a final byte.
                Some(b'[' | b'O') => match bytes[i + 2..]
                    .iter()
                    .position(|b| (0x40..=0x7e).contains(b))
                {
                    Some(p) => {
                        let key = match bytes[i + 2 + p] {
                            b'A' => Key::Up,
                            b'B' => Key::Down,
                            b'C' => Key::Right,
                            b'D' => Key::Left,
                            b'Z' => Key::Up,
                            _ => Key::Ignored,
                        };
                        (key, p + 3)
                    }
                    None => break,
                },
                _ => (Key::Esc, 1),
            },
            b'\r' => (
                Key::Enter,
                if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                },
            ),
            b'\n' => (Key::Enter, 1),
            0x7f | 0x08 => (Key::Backspace, 1),
            0x03 => (Key::Interrupt, 1),
            0x04 => (Key::End, 1),
            0x15 => (Key::ClearLine, 1),
            0x17 => (Key::DeleteWord, 1),
            b'\t' => (Key::Tab, 1),
            0x00..=0x1f => (Key::Ignored, 1),
            lead => {
                let len = match lead {
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf7 => 4,
                    _ => 1,
                };
                if i + len > bytes.len() {
                    break;
                }
                match std::str::from_utf8(&bytes[i..i + len])
                    .ok()
                    .and_then(|s| s.chars().next())
                {
                    Some(c) => (Key::Char(c), len),
                    None => (Key::Ignored, 1),
                }
            }
        };
        keys.push(key);
        i += len;
    }
    (keys, i)
}

/// The terminal itself: its size, its modes, and keys one at a time.
mod term {
    use std::collections::VecDeque;
    use std::io::{self, Write};
    use std::sync::Mutex;

    use super::Key;

    /// The terminal's settings before ssboot changed them.
    static SAVED: Mutex<Option<libc::termios>> = Mutex::new(None);

    /// Columns and rows. Without an answer, an iPhone's upright: a guess too
    /// narrow only wraps sooner, one too wide breaks the redraw.
    pub fn size() -> (usize, usize) {
        // SAFETY: TIOCGWINSZ fills the winsize it's given, and nothing else.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            return (ws.ws_col as usize, ws.ws_row as usize);
        }
        let env = |key: &str| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|n| *n > 0)
        };
        (env("COLUMNS").unwrap_or(40), env("LINES").unwrap_or(24))
    }

    pub fn write(text: &str) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn get() -> Option<libc::termios> {
        // SAFETY: tcgetattr fills the termios it's given.
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        (unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut t) } == 0).then_some(t)
    }

    fn set(t: &libc::termios) {
        // SAFETY: a termios read back from this terminal, maybe with flags off.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t) };
    }

    /// For the live screen: keys typed while ssboot works don't echo into
    /// it, and a panic leaves the terminal as it was. False when the terminal
    /// has no settings to change.
    pub fn start() -> bool {
        let Some(saved) = get() else { return false };
        *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(saved);
        let mut quiet = saved;
        quiet.c_lflag &= !(libc::ECHO | libc::ECHONL);
        set(&quiet);
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            super::on_panic();
            default_hook(info);
        }));
        true
    }

    /// The settings as they were, and the cursor shown.
    pub fn restore() {
        if let Some(saved) = SAVED.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            set(saved);
        }
        write("\x1b[?25h");
    }

    /// Keys one at a time while it lives: no line editing, no echo, and
    /// Ctrl-C as a key rather than a signal.
    pub struct Keys {
        before: libc::termios,
        pending: Vec<u8>,
        queue: VecDeque<Key>,
    }

    impl Keys {
        pub fn open() -> Option<Keys> {
            let before = get()?;
            let mut raw = before;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHONL | libc::ISIG | libc::IEXTEN);
            raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::INLCR);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            set(&raw);
            // What was typed before the question isn't its answer.
            // SAFETY: drops pending input on this terminal.
            unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH) };
            Some(Keys {
                before,
                pending: Vec::new(),
                queue: VecDeque::new(),
            })
        }

        pub fn next(&mut self) -> Key {
            loop {
                if let Some(key) = self.queue.pop_front() {
                    return key;
                }
                let mut buf = [0u8; 256];
                // SAFETY: reads at most buf.len() bytes into buf.
                let n =
                    unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
                if n < 0 {
                    if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Key::End;
                }
                if n == 0 {
                    return Key::End;
                }
                self.pending.extend_from_slice(&buf[..n as usize]);
                let (keys, used) = super::parse_keys(&self.pending);
                self.pending.drain(..used);
                self.queue.extend(keys);
            }
        }
    }

    impl Drop for Keys {
        fn drop(&mut self) {
            set(&self.before);
        }
    }
}

/// tracing's output on the live screen: each line printed above the live
/// area, so a library's warning doesn't tear it.
pub struct LogLines;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogLines {
    type Writer = LogLine;

    fn make_writer(&'a self) -> LogLine {
        LogLine(Vec::new())
    }
}

pub struct LogLine(Vec<u8>);

impl Write for LogLine {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for LogLine {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0);
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            log(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_count_columns_not_bytes() {
        assert_eq!(width("ssboot"), 6);
        assert_eq!(width("✓ ◆ ○ ━ ╸ ─ › … ⠋"), 17);
        assert_eq!(width("日本"), 4);
        assert_eq!(width("e\u{301}"), 1);
    }

    #[test]
    fn a_line_is_cut_to_the_screen_with_an_ellipsis() {
        let line = Line::new()
            .add(BOLD, "Apple ID  ")
            .add(NONE, "signing in through new.ani.sidestore.io");
        let cut = line.render(20, false);
        assert_eq!(cut, "Apple ID  signing i…");
        assert_eq!(width(&cut), 20);
        assert_eq!(
            line.render(80, false),
            "Apple ID  signing in through new.ani.sidestore.io"
        );
        // Styles close before the cut.
        let styled = line.render(12, true);
        assert!(styled.starts_with("\x1b[1mApple ID  \x1b[0m"), "{styled:?}");
        assert!(styled.ends_with('…'));
    }

    #[test]
    fn words_wrap_and_long_ones_are_cut() {
        assert_eq!(
            wrap("Keep Minis open until ssboot is done", 16),
            ["Keep Minis open", "until ssboot is", "done"]
        );
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(
            wrap("https://example.org/a/long/path", 10),
            ["https://ex", "ample.org/", "a/long/pat", "h"]
        );
        for line in wrap(
            "The iPhone opened a tunnel port but 10.7.0.1:61000 can't be reached.",
            24,
        ) {
            assert!(width(&line) <= 24, "{line}");
        }
    }

    #[test]
    fn a_long_answer_keeps_its_end_in_view() {
        assert_eq!(keep_end("me@icloud.com", 20), "me@icloud.com");
        assert_eq!(
            keep_end("someone.with.a.long.name@icloud.com", 12),
            "…@icloud.com"
        );
    }

    #[test]
    fn keys_parse_from_what_the_terminal_sends() {
        let (keys, used) = parse_keys(b"ab\x7f\r\x1b[A\x1b[B\x1bOC\x03");
        assert_eq!(
            keys,
            [
                Key::Char('a'),
                Key::Char('b'),
                Key::Backspace,
                Key::Enter,
                Key::Up,
                Key::Down,
                Key::Right,
                Key::Interrupt
            ]
        );
        assert_eq!(used, 14);
        // A lone escape, and CRLF as one return.
        assert_eq!(parse_keys(b"\x1b").0, [Key::Esc]);
        assert_eq!(parse_keys(b"\r\n").0, [Key::Enter]);
        // UTF-8 whole, and a sequence cut short waits for the rest.
        assert_eq!(
            parse_keys("é•".as_bytes()).0,
            [Key::Char('é'), Key::Char('•')]
        );
        assert_eq!(parse_keys(b"x\x1b["), (vec![Key::Char('x')], 1));
        assert_eq!(parse_keys(&"é".as_bytes()[..1]), (vec![], 0));
    }

    #[test]
    fn code_boxes_fill_in_two_groups() {
        let (line, col) = code_boxes("", 6);
        assert_eq!(line.render(40, false), "_ _ _  _ _ _");
        assert_eq!(col, 0);
        let (line, col) = code_boxes("4815", 6);
        assert_eq!(line.render(40, false), "4 8 1  5 _ _");
        assert_eq!(col, 9);
        let (_, col) = code_boxes("481516", 6);
        assert_eq!(col, 12);
    }

    #[test]
    fn the_progress_bar_fits_its_line() {
        let bar = Bar {
            done: 31 * 1_048_576,
            total: 51 * 1_048_576,
            bytes: true,
            started: Instant::now(),
        };
        for max in [30, 45, 60, 120] {
            let line = bar.line(max);
            assert!(line.width() <= max, "{max}: {}", line.render(200, false));
            assert!(line.render(200, false).contains("60%"));
        }
        let wide = bar.line(60).render(200, false);
        assert!(wide.contains("31.0/51.0 MB"), "{wide}");
        let done = Bar {
            done: 100,
            total: 100,
            bytes: false,
            started: Instant::now(),
        };
        assert!(!done.line(40).render(200, false).contains('─'));
    }

    #[test]
    fn a_box_is_as_wide_as_its_edges() {
        let lines = boxed(
            false,
            GOOD,
            "✓ Done",
            "3:41",
            &[
                "SideStore is installed.",
                "Free accounts' apps last 7 days.",
            ],
            46,
        );
        let widths: Vec<usize> = lines.iter().map(|l| width(l)).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "{lines:#?}");
        assert!(widths[0] <= 45);
        assert!(lines[0].contains("✓ Done") && lines[0].contains("3:41"));
        assert!(lines.last().unwrap().trim_start().starts_with('╰'));
    }

    #[test]
    fn hanging_lines_line_up_under_the_text() {
        let lead = Line::new()
            .add(NONE, "  ")
            .add(NONE, "✓ ")
            .add(NONE, pad("Connect", NAME_WIDTH));
        let lines = hanging(
            &lead,
            NONE,
            "Tom's iPhone, an iPhone 17 Pro on iOS 27.1",
            30,
            false,
        );
        assert_eq!(lines[0], "  ✓ Connect   Tom's iPhone, an");
        assert!(
            lines[1..]
                .iter()
                .all(|l| l.starts_with(&" ".repeat(TEXT_COLUMN))),
            "{lines:#?}"
        );
    }

    #[test]
    fn a_long_status_takes_two_lines_then_is_cut() {
        let lead = Line::new()
            .add(NONE, "  ⠋ ")
            .add(NONE, pad("Pairing", NAME_WIDTH));
        let text = "rootshell is offering the pairing file it made";
        let lines: Vec<String> = wrapped(lead.clone(), None, NONE, text, Some(" · 0:07"), 42, 2)
            .iter()
            .map(|l| l.render(42, false))
            .collect();
        assert_eq!(
            lines,
            [
                "  ⠋ Pairing   rootshell is offering the",
                "              pairing file it made · 0:07"
            ]
        );
        // No room for the timer: it waits.
        let narrow = wrapped(lead.clone(), None, NONE, text, Some(" · 0:07"), 40, 2);
        assert!(!narrow[1].render(40, false).contains("0:07"));
        let long = "one two three four five six seven eight nine ten eleven twelve thirteen";
        let lines = wrapped(lead, None, NONE, long, None, 30, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].render(30, false).ends_with('…'));
        // A prompt's hint keeps its rail on every line.
        let rail_lead = rail().add(NONE, " ".repeat(NAME_WIDTH));
        let hint = wrapped(rail_lead.clone(), Some(rail_lead), DIM, long, None, 34, 2);
        assert!(hint.iter().all(|l| l.render(34, false).starts_with("  │ ")));
    }

    #[test]
    fn names_pad_to_their_column() {
        assert_eq!(pad("Pairing", NAME_WIDTH), "Pairing   ");
        assert_eq!(width(&pad("Handover", NAME_WIDTH)), NAME_WIDTH);
        // A name as long as the column still gets a space after it.
        assert_eq!(pad("Developers", NAME_WIDTH), "Developers ");
    }

    #[test]
    fn the_credit_ends_the_live_area_while_there_is_room() {
        let credit = format!("  {CREDIT}");
        let mut ui = Ui::new();
        ui.plan = vec!["Check", "Pairing", "Connect"];
        ui.active = Some(0);
        ui.status = "reaching GitHub".to_string();
        let (lines, _) = ui.compose(46, 40);
        assert_eq!(lines.len(), 5, "{lines:#?}");
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], credit);
        // Under a prompt, which keeps its cursor.
        ui.dialog = Some(Dialog {
            lines: vec![rail().add(BOLD, pad("Code", NAME_WIDTH))],
            cursor: Some((0, TEXT_COLUMN)),
        });
        let (lines, cursor) = ui.compose(46, 40);
        assert_eq!(lines.last(), Some(&credit));
        assert_eq!(cursor, Some((1, TEXT_COLUMN)));
        ui.dialog = None;
        // Alone, after the header or a box: their blank line is its own.
        ui.active = None;
        assert_eq!(ui.compose(46, 40).0, std::slice::from_ref(&credit));
        // Alone after a line that isn't blank: one of its own.
        ui.after_blank = false;
        assert_eq!(ui.compose(46, 40).0, [String::new(), credit.clone()]);
        // A screen too short for it keeps the run's lines.
        ui.active = Some(0);
        assert_eq!(ui.compose(46, 6).0.last(), Some(&credit));
        for rows in [3, 4, 5] {
            let (lines, _) = ui.compose(46, rows);
            assert!(!lines.contains(&credit), "{rows}: {lines:#?}");
            assert_eq!(lines.len(), 3.min(rows - 1), "{rows}: {lines:#?}");
        }
        // On a narrow one it's cut like any other line (to 20 columns at least).
        let (lines, _) = ui.compose(20, 40);
        assert!(lines.iter().all(|l| width(l) <= 20), "{lines:#?}");
        assert_eq!(lines.last().map(String::as_str), Some("  A project by Friz…"));
    }

    #[test]
    fn clocks_read_as_minutes_and_seconds() {
        assert_eq!(clock(7), "0:07");
        assert_eq!(clock(221), "3:41");
        assert_eq!(clock(3729), "1:02:09");
    }
}
