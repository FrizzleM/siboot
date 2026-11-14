//! ssboot's live screen through a made-up run, for working on the UI without
//! an iPhone. Nothing here talks to a device or to Apple:
//!
//! ```sh
//! cargo run --example screens                  # an install, pairing in Minis
//! cargo run --example screens -- rootshell     # taking rootshell's pairing
//! cargo run --example screens -- stop          # one that stops at Connect
//! cargo run --example screens -- developer-mode
//! cargo run --example screens -- --plain       # the same, as plain lines
//! cargo run --example screens -- --web         # the same, as the page
//! ```
//!
//! A run that stops at Connect then asks whether to copy the log, as ssboot
//! does; on a Mac, yes puts it on the clipboard with `pbcopy`.
//!
//! With `--web` the run is served as ssboot serves it, on the loopback, and
//! its address printed; it starts once a browser has the page open, and the
//! questions are answered there. `--open` opens it with macOS's `open`.
//!
//! The codes it asks for are 481516 (transfer) and 123456 (Apple's). The
//! pairing code it shows is 975 088.

#![allow(dead_code)]

#[path = "../src/ui.rs"]
mod ui;
#[path = "../src/web.rs"]
mod web;

use std::thread::sleep;
use std::time::Duration;

fn wait(ms: u64) {
    sleep(Duration::from_millis(ms));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let page = if has("--web") {
        Some(web::start().expect("a loopback port for the page"))
    } else {
        None
    };
    ui::init(false, has("--plain") || page.is_some());
    if let Some(url) = &page {
        web::on_stop(|| ui::stop_now("Stopped from the page."));
        println!("The page: {url}");
        if has("--open") {
            let _ = std::process::Command::new("open").arg(url).status();
        }
        web::wait_for_page(Duration::from_secs(600));
    }
    let devmode = has("developer-mode");
    let stop = has("stop");
    let code = if run(devmode, stop, has("rootshell")) { 0 } else { 1 };
    if stop && ui::confirm("copy the log to the clipboard?", true) {
        let log = ui::recorded().join("\n");
        let copied = std::process::Command::new("pbcopy")
            .env("LANG", "en_US.UTF-8")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                child.stdin.take().map(|mut i| i.write_all(log.as_bytes()));
                child.wait()
            });
        match copied {
            Ok(status) if status.success() => ui::ok(&format!(
                "Copied the log ({} lines). Paste it where you ask for help.",
                log.lines().count()
            )),
            _ => ui::warn("Couldn't copy the log: there's no pbcopy here."),
        }
        ui::note("It's also in ~/.ssboot/ssboot.log.");
    }
    ui::close();
    std::process::exit(code);
}

fn run(devmode: bool, stop: bool, rootshell: bool) -> bool {
    let steps = [
        "Check", "Pairing", "Connect", "Apple ID", "Sign", "Install", "Handover",
    ];
    if devmode {
        ui::header("0.3.0", "Shows Developer Mode, or turns it on.");
        ui::plan(&["Check", "Pairing", "Connect", "Dev Mode"], 4);
    } else {
        ui::header("0.3.0", "Installs SideStore on this iPhone.");
        ui::plan(&steps, 7);
    }

    ui::step(1, "Checking LocalDevVPN, this iPhone and the internet");
    for (what, took, answer) in [
        (
            "reaching github.com",
            700,
            "github.com answers, and SideStore's nightly is there",
        ),
        (
            "reaching Apple's sign-in server",
            600,
            "Apple's sign-in server answers",
        ),
        (
            "reaching this iPhone at 10.7.0.1:49152",
            500,
            "this iPhone answers on 10.7.0.1:49152",
        ),
    ] {
        ui::activity(what);
        wait(took);
        ui::ok(answer);
    }
    ui::done("online, and the iPhone answers");

    ui::step(2, "Pairing this iPhone with ssboot");
    ui::activity("checking that Open Minis can pair this iPhone");
    wait(500);
    if rootshell {
        if !take_from_rootshell() {
            return false;
        }
    } else if !pair_here() {
        return false;
    }
    ui::step(3, "Connecting to this iPhone through LocalDevVPN");
    for (what, took) in [
        ("greeting the iPhone", 300),
        ("checking the pairing with it", 500),
        ("asking the iPhone for a tunnel listener", 600),
        ("starting the encrypted tunnel", 1400),
        ("reading the iPhone's services", 900),
        ("asking lockdownd about this iPhone", 500),
    ] {
        ui::activity(what);
        wait(took);
        if stop && what == "starting the encrypted tunnel" {
            ui::failure(
                "Timed out while starting the encrypted tunnel.",
                "no answer within 20 s",
            );
            return false;
        }
    }
    ui::ok("Frizzle's iPhone — iPhone18,1 on iOS 27.1");
    ui::done("Frizzle's iPhone · iOS 27.1");

    if devmode {
        ui::step(4, "Asking this iPhone for Developer Mode");
        ui::activity("asking whether Developer Mode is on");
        wait(700);
        ui::status("Developer Mode is off");
        ui::warn(
            "Without a passcode, this iPhone restarts right away to turn Developer Mode on, and \
             Minis stops with it. Once you unlock it, tap Turn On in the alert iOS shows.",
        );
        ui::note("With a passcode, iOS refuses, and ssboot shows the switch in Settings instead.");
        if !ui::confirm("turn it on now?", false) {
            ui::failure(
                "Nothing was sent, so Developer Mode stays off. `ssboot developer-mode` without \
                 --turn-on only shows it in Settings, and restarts nothing.",
                "",
            );
            return false;
        }
        ui::activity("connecting to AMFI on the iPhone");
        wait(800);
        ui::done("set to turn on");
        ui::finish(
            "Developer Mode is turning on",
            &[
                "This iPhone restarts now, and Minis stops with it.",
                "Once you unlock it, tap Turn On.",
            ],
        );
        return true;
    }

    ui::step(4, "Signing in to your Apple Account");
    let Some(email) = ui::ask_text(
        "sign in to your Apple Account",
        "Email",
        "your Apple Account",
        "Apple Account email: ",
    ) else {
        ui::failure("No Apple Account given.", "");
        return false;
    };
    let password = ui::ask_secret(
        "sign in to your Apple Account",
        "Password",
        "it goes only to Apple",
        "Password (not shown; it goes only to Apple): ",
    );
    if !matches!(password, Ok(Some(ref p)) if !p.is_empty()) {
        ui::failure("No password given.", "");
        return false;
    }
    ui::status("signing in through new.ani.sidestore.io");
    wait(1500);
    ui::warn("new.ani.sidestore.io didn't work: anisette server answered 503 Service Unavailable");
    ui::status("signing in through mac-ani.jkcoxson.com");
    wait(1200);
    let mut hints = vec![ui::Hint::Plain("sent to your Apple devices".into())];
    loop {
        match ui::ask_code_or_more("enter the code Apple sent", "Code", &hints) {
            Some(ui::CodeAnswer::Code(code)) if code == "123456" => break,
            Some(ui::CodeAnswer::Code(_)) => {
                hints.retain(|hint| !matches!(hint, ui::Hint::Warning(_)));
                hints.insert(
                    0,
                    ui::Hint::Warning("Apple: Incorrect verification code.".into()),
                );
            }
            Some(ui::CodeAnswer::More) => {
                let ways = [
                    "Text +1 (•••) •••-••42".to_string(),
                    "Call +1 (•••) •••-••42".to_string(),
                    "Send it to your Apple devices".to_string(),
                    "Send the same again".to_string(),
                    "Cancel the sign-in".to_string(),
                ];
                match ui::choose("how should Apple send the code?", &ways) {
                    Some(4) => {
                        ui::failure("Sign-in cancelled.", "");
                        return false;
                    }
                    Some(i) => {
                        hints = vec![ui::Hint::Plain(match i {
                            0 => "texted to +1 (•••) •••-••42".into(),
                            1 => "Apple is calling +1 (•••) •••-••42".into(),
                            _ => "sent to your Apple devices".into(),
                        })];
                        ui::status("asking Apple for a new code");
                        wait(900);
                    }
                    None => {}
                }
            }
            Some(ui::CodeAnswer::Cancel) => {
                ui::failure("Sign-in cancelled.", "");
                return false;
            }
            None => match ui::ask("Code: ") {
                Some(code) if code == "123456" => break,
                Some(_) => {}
                None => {
                    ui::failure("Sign-in cancelled.", "");
                    return false;
                }
            },
        }
        ui::status("checking the code with Apple");
        wait(800);
    }
    ui::status("checking the code with Apple");
    wait(900);
    ui::ok(&format!("signed in as {email} — team Frizzle (ABCDE12345)"));
    ui::done("team Frizzle (ABCDE12345)");

    ui::step(5, "Signing SideStore for this iPhone");
    ui::status("downloading SideStore");
    let total: u64 = 30_681_634;
    for i in 0..=60u64 {
        ui::progress("downloaded", total * i / 60, total);
        wait(45);
    }
    ui::progress_end();
    ui::ok("downloaded SideStore (29.3 MB)");
    ui::activity("checking it's SideStore");
    wait(600);
    ui::ok("it's SideStore 0.7.0-20261027.1507");
    ui::status("signing; this takes a while here");
    wait(4200);
    ui::ok("signed SideStore as com.SideStore.SideStore.ABCDE12345");
    ui::detail(
        "this build asks for a password before it takes a certificate file, so it's given none, \
         as SideInstaller does",
    );
    ui::done("SideStore 0.7.0-20261027.1507");

    ui::step(6, "Installing it");
    let total: u64 = 46 * 1_048_576 + 600_000;
    ui::status("uploading 158 files (46.6 MB)");
    for i in 0..=80u64 {
        ui::progress("uploaded", total * i / 80, total);
        wait(50);
    }
    ui::progress_end();
    ui::status("checking every file arrived whole");
    wait(700);
    ui::status("installing");
    for percent in (0..=100).step_by(4) {
        ui::percent("installed", percent);
        wait(70);
    }
    ui::progress_end();
    ui::ok("SideStore installed (com.SideStore.SideStore.ABCDE12345)");
    ui::done("installed · 46.6 MB, 158 files");

    ui::step(7, "Handing SideStore the pairing");
    ui::activity("opening SideStore's container");
    wait(500);
    ui::activity("writing the pairing files into it");
    wait(900);
    ui::ok("pairing files written into SideStore (2 of them, each read back)");
    ui::activity("setting SideStore to load them");
    wait(500);
    ui::ok("SideStore loads the RPPairing file from its first launch");
    ui::ok(
        "SideStore's anisette set as its users get past -80046: on-device anisette off, \
         auto-rotation off, server SideStore (New)",
    );
    ui::done("SideStore has the pairing");

    ui::link("Open SideInstaller", "sideinstaller://");
    ui::finish(
        "SideStore is installed",
        &[
            "Open SideStore from your Home Screen. The first time, iOS asks you to trust your \
             Apple Account: Settings › General › VPN & Device Management.",
            "It has this iPhone's pairing already. Sign in to it with the same Apple Account, and \
             keep LocalDevVPN connected while it installs or refreshes apps.",
            "If it offers to resign itself when you sign in, let it.",
            "Free accounts' apps last 7 days: refresh them in SideStore before then.",
        ],
    );
    true
}

/// Step 2 as Minis 1.16 does it: ssboot pairs this iPhone with itself.
fn pair_here() -> bool {
    ui::ok("Open Minis can pair this iPhone");
    ui::status("start pairing when you're ready");
    if !ui::get_ready(
        "Pair this iPhone with ssboot",
        &[
            "Open Settings › Privacy & Security › Developer Mode, and scroll to the bottom.",
            "Tap Pair with ssboot. It can take a few seconds to show.",
            "Enter your iPhone passcode, then the 6-digit code ssboot shows. It comes as a \
             notification too.",
            "Come back to Minis once Settings says it's paired.",
        ],
        &["When you start, a player opens over Minis. It plays silence, which keeps Minis \
           running while you're in Settings: leave it open. ssboot closes it once you're back."],
        "Start pairing",
    ) {
        ui::failure("Stopped before pairing.", "");
        return false;
    }
    ui::ok("Pair with ssboot is up in Settings");
    ui::status("waiting for Pair with ssboot in Settings (up to 10 minutes)");
    wait(2500);
    ui::status("Settings connected: waiting for it to ask for a code");
    wait(600);
    ui::status("waiting for the code to be typed into Settings");
    ui::show_code("975088", "Type it into Settings. It came as a notification too.");
    wait(5000);
    ui::code_gone();
    ui::status("paired: go back to Minis to carry on");
    wait(1500);
    ui::ok("paired with Frizzle's iPhone (iPhone18,1)");
    ui::activity("checking the new pairing with this iPhone");
    wait(700);
    ui::ok("this iPhone accepts the new pairing");
    ui::done("paired here in Minis");
    true
}

/// Step 2 with ssboot-pair offering its file from rootshell.
fn take_from_rootshell() -> bool {
    ui::ok("rootshell is offering its file");
    let mut tries = 3;
    loop {
        let Some(code) = ui::ask_code(
            "rootshell is offering its file",
            "Code",
            "type the transfer code it shows",
            "Transfer code shown in rootshell: ",
        ) else {
            ui::failure("Stopped at the transfer code.", "");
            return false;
        };
        ui::activity("sending the code to rootshell");
        wait(400);
        if code == "481516" {
            break;
        }
        tries -= 1;
        if tries == 0 {
            ui::failure("The transfer code didn't match. Run ssboot again and type the code rootshell shows.", "");
            return false;
        }
        ui::warn(&format!(
            "That isn't the code rootshell shows ({tries} more {}).",
            if tries == 1 { "try" } else { "tries" }
        ));
    }
    ui::ok("took the pairing file from rootshell (1184 bytes)");
    ui::activity("checking it with this iPhone");
    wait(700);
    ui::done("taken from rootshell");
    true
}
