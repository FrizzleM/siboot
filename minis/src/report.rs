//! The log siboot offers to copy when a run stops: what the screen showed,
//! the libraries' own log lines and the underlying error, as plain text.
//!
//! It goes to the clipboard through Open Minis's `apple-clipboard set`, which
//! reads the text on stdin (Minis's terminal ignores OSC 52), and is saved to
//! ~/.siboot/siboot.log either way. Before either, the Apple Account's email
//! and long identifiers (the iPhone's UDID, anisette identities) are cut short:
//! the log is meant to be pasted where you ask for help. Passwords and codes
//! never reach it (`ui::record_answer`).

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};

use crate::fail::Fail;
use crate::{ui, Args, Command, VERSION};

pub const LOG_FILE: &str = "siboot.log";

/// After a failure the user didn't choose: save the log, and on a terminal or
/// the page ask whether to copy it.
pub fn after_failure(args: &Args, f: &Fail) {
    if f.by_user {
        return;
    }
    let text = compose(args);
    let path = args.home.join(LOG_FILE);
    let saved = std::fs::write(&path, &text).is_ok();
    let shown = shown_path(&path);
    let asks = crate::web::page().is_some()
        || (std::io::stdin().is_terminal() && std::io::stdout().is_terminal());
    if !asks {
        if saved {
            ui::note(&format!("The run's log is in {shown}."));
        }
        return;
    }
    if !ui::confirm("copy the log to the clipboard?", true) {
        if saved {
            ui::note(&format!("The log is in {shown}."));
        }
        return;
    }
    match copy(&text) {
        Ok(()) => {
            let lines = text.lines().count();
            ui::ok(&format!(
                "Copied the log ({lines} lines). Paste it where you ask for help."
            ));
        }
        Err(why) => ui::warn(&format!("Couldn't copy the log: {why}.")),
    }
    if saved {
        ui::note(&format!("It's also in {shown}."));
    }
}

/// The log: a header, then the run, cleaned.
fn compose(args: &Args) -> String {
    let command = match args.command {
        Command::All => "install",
        Command::Check => "check",
        Command::Connect => "connect",
        Command::DeveloperMode => "developer-mode",
    };
    let mut out = format!("siboot {VERSION} log · {command}");
    let flags = flags_given(std::env::args().skip(1));
    if !flags.is_empty() {
        out.push_str(&format!(" · {}", flags.join(" ")));
    }
    out.push('\n');
    out.push_str(&format!("{}\n", system()));
    out.push_str("-----\n");
    for line in ui::recorded() {
        out.push_str(&line);
        out.push('\n');
    }
    redact(&out, args.apple_id.as_deref())
}

/// The options given, without their values: those can be an email or a path.
fn flags_given(args: impl Iterator<Item = String>) -> Vec<String> {
    args.filter(|a| a.starts_with('-'))
        .map(|a| a.split('=').next().unwrap_or("").to_string())
        .collect()
}

/// The kernel iSH says it is, and the terminal.
fn system() -> String {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|k| k.trim().to_string())
        .unwrap_or_else(|_| "?".into());
    let term = std::env::var("TERM").unwrap_or_else(|_| "?".into());
    format!(
        "{} {} · kernel {kernel} · TERM {term}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

/// `~/.siboot/siboot.log` rather than `/root/.siboot/siboot.log`.
fn shown_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Onto the clipboard: Open Minis's `apple-clipboard`, else `pbcopy` (a Mac,
/// running the screens example).
fn copy(text: &str) -> Result<(), String> {
    match pipe_to("apple-clipboard", &["set"], text) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            pipe_to("pbcopy", &[], text).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "there's no apple-clipboard here (it comes with Open Minis)".to_string()
                } else {
                    e.to_string()
                }
            })
        }
        other => other.map_err(|e| e.to_string()),
    }
}

fn pipe_to(program: &str, args: &[&str], text: &str) -> std::io::Result<()> {
    let mut process = Process::new(program);
    if program == "pbcopy" {
        // Else it reads the text as Mac Roman.
        process.env("LANG", "en_US.UTF-8");
    }
    let mut child = process
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("{program} answered {status}")))
    }
}

/// The email cut to its first letter and domain, and any long run of hex
/// digits (a UDID, an anisette identity, a key) to its ends.
fn redact(text: &str, apple_id: Option<&str>) -> String {
    let mut text = text.to_string();
    if let Some(id) = apple_id.map(str::trim).filter(|id| !id.is_empty()) {
        text = text.replace(id, &short_email(id));
    }
    let text = emails(&text);
    long_hex(&text)
}

fn short_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}…@{domain}")
        }
        None => "…".into(),
    }
}

/// Every `name@domain.tld` in `text`, shortened.
fn emails(text: &str) -> String {
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut start = i;
            while start > 0 && local(chars[start - 1]) {
                start -= 1;
            }
            let mut end = i + 1;
            while end < chars.len() && domain(chars[end]) {
                end += 1;
            }
            let host: String = chars[i + 1..end].iter().collect();
            let host = host.trim_end_matches('.');
            if start < i && host.contains('.') {
                // The local part is already in `out`: take it back.
                for _ in start..i {
                    out.pop();
                }
                let email: String = chars[start..i + 1 + host.chars().count()].iter().collect();
                out.push_str(&short_email(&email));
                i += 1 + host.chars().count();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Runs of 20 or more hex digits (dashes allowed between), kept to their
/// first and last four.
fn long_hex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let digits = run.chars().filter(char::is_ascii_hexdigit).count();
        let has_digit = run.chars().any(|c| c.is_ascii_digit());
        if digits >= 20 && has_digit {
            let trimmed = run.trim_matches('-');
            let head: String = trimmed.chars().take(4).collect();
            let tail: String = trimmed.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            let lead = &run[..run.len() - run.trim_start_matches('-').len()];
            let trail = &run[run.trim_end_matches('-').len()..];
            out.push_str(&format!("{lead}{head}…{tail}{trail}"));
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in text.chars() {
        if c.is_ascii_hexdigit() || (c == '-' && !run.is_empty()) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apple_account_is_cut_to_a_letter_and_its_domain() {
        let text = "signing in as jane.doe+x@icloud.com, then mail@example.org.";
        assert_eq!(
            redact(text, Some("jane.doe+x@icloud.com")),
            "signing in as j…@icloud.com, then m…@example.org."
        );
    }

    #[test]
    fn an_at_sign_alone_is_left_as_it_is() {
        assert_eq!(emails("a @ b, x@y, @home"), "a @ b, x@y, @home");
    }

    #[test]
    fn a_udid_keeps_only_its_ends() {
        assert_eq!(
            long_hex("UDID 00008110-001A2B3C4D5E801E here"),
            "UDID 0000…801E here"
        );
        assert_eq!(
            long_hex("identity 3f1c9a0d5e7b2c4f6a8d0e1f3a5b7c9d1e2f3a4b."),
            "identity 3f1c…3a4b."
        );
    }

    #[test]
    fn short_numbers_words_and_addresses_stay() {
        let text = "port 61207, iOS 27.0, 10.7/11.3 MB, deadbeef, fe80::1, 2026-11-12";
        assert_eq!(long_hex(text), text);
    }

    #[test]
    fn options_are_kept_without_their_values() {
        let given = ["developer-mode", "--turn-on", "--apple-id", "me@x.com", "--home=/tmp/x", "-v"];
        assert_eq!(
            flags_given(given.iter().map(|s| s.to_string())),
            ["--turn-on", "--apple-id", "--home", "-v"]
        );
    }

    #[test]
    fn the_home_folder_reads_as_a_tilde() {
        if let Some(home) = std::env::var_os("HOME") {
            let path = PathBuf::from(home).join(".siboot").join(LOG_FILE);
            assert_eq!(shown_path(&path), "~/.siboot/siboot.log");
        }
    }
}
