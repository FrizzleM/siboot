//! siboot's live screen through a made-up run, for working on the UI without
//! an iPhone. Nothing here talks to a device or to Apple:
//!
//! ```sh
//! cargo run --example screens                  # an install
//! cargo run --example screens -- stop          # one that stops at Connect
//! cargo run --example screens -- developer-mode
//! cargo run --example screens -- --plain       # the same, as plain lines
//! ```
//!
//! The codes it asks for are 481516 (transfer) and 123456 (Apple's).

#![allow(dead_code)]

#[path = "../src/ui.rs"]
mod ui;

use std::thread::sleep;
use std::time::Duration;

fn wait(ms: u64) {
    sleep(Duration::from_millis(ms));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    ui::init(false, has("--plain"));
    let devmode = has("developer-mode");
    let code = if run(devmode, has("stop")) { 0 } else { 1 };
    ui::close();
    std::process::exit(code);
}

fn run(devmode: bool, stop: bool) -> bool {
    let steps = [
        "Check", "Pairing", "Connect", "Apple ID", "Sign", "Install", "Handover",
    ];
    if devmode {
        ui::header("0.15.0", "Shows Developer Mode, or turns it on.");
        ui::plan(&["Check", "Pairing", "Connect", "Dev Mode"], 4);
    } else {
        ui::header("0.15.0", "Installs SideInstaller on this iPhone.");
        ui::plan(&steps, 7);
    }

    ui::step(1, "Checking LocalDevVPN, this iPhone and the internet");
    for (what, took, answer) in [
        (
            "reaching GitHub",
            700,
            "GitHub answers, and the SideInstaller release is there",
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

    ui::step(2, "Taking the pairing from rootshell");
    ui::activity("asking rootshell for its pairing file");
    wait(600);
    ui::ok("rootshell is offering its file");
    let mut tries = 3;
    loop {
        let Some(code) = ui::ask_code(
            "rootshell is offering its file",
            "Code",
            "type the transfer code it shows",
            "Transfer code shown in rootshell: ",
        ) else {
            ui::failure("rootshell isn't offering a pairing file.", "");
            return false;
        };
        ui::activity("sending the code to rootshell");
        wait(400);
        if code == "481516" {
            break;
        }
        tries -= 1;
        if tries == 0 {
            ui::failure("The transfer code didn't match. Run siboot again and type the code rootshell shows.", "");
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
        ui::note("With a passcode, iOS refuses, and siboot shows the switch in Settings instead.");
        if !ui::confirm("turn it on now?", false) {
            ui::failure(
                "Nothing was sent, so Developer Mode stays off. `siboot developer-mode` without \
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
    let mut hints = vec![
        ui::Hint::Plain("sent to your Apple devices".into()),
        ui::Hint::Plain("o: other ways · esc: cancel".into()),
    ];
    loop {
        match ui::ask_code_or_more("enter the code Apple sent", "Code", &hints) {
            Some(ui::CodeAnswer::Code(code)) if code == "123456" => break,
            Some(ui::CodeAnswer::Code(_)) => {
                hints.insert(
                    0,
                    ui::Hint::Warning("Apple: Incorrect verification code.".into()),
                );
                hints.truncate(3);
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
                        hints = vec![
                            ui::Hint::Plain(match i {
                                0 => "texted to +1 (•••) •••-••42".into(),
                                1 => "Apple is calling +1 (•••) •••-••42".into(),
                                _ => "sent to your Apple devices".into(),
                            }),
                            ui::Hint::Plain("o: other ways · esc: cancel".into()),
                        ];
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

    ui::step(5, "Signing SideInstaller for this iPhone");
    ui::status("downloading SideInstaller");
    let total: u64 = 24 * 1_048_576 + 400_000;
    for i in 0..=60u64 {
        ui::progress("downloaded", total * i / 60, total);
        wait(45);
    }
    ui::progress_end();
    ui::ok("downloaded SideInstaller (24.4 MB)");
    ui::status("signing; this takes a while here");
    wait(4200);
    ui::ok("signed SideInstaller as com.frizzlem.sideinstaller.ABCDE12345");
    ui::done("SideInstaller 1.7.0 signed");

    ui::step(6, "Installing it");
    let total: u64 = 51 * 1_048_576;
    ui::status("uploading 412 files (51.0 MB)");
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
    ui::ok("SideInstaller installed (com.frizzlem.sideinstaller.ABCDE12345)");
    ui::done("installed · 51.0 MB, 412 files");

    ui::step(7, "Handing SideInstaller the pairing");
    ui::activity("writing the pairing file into it");
    wait(900);
    ui::done("SideInstaller has the pairing");

    ui::finish(
        "SideInstaller is installed",
        &[
            "Open it from your Home Screen. The first time, iOS asks you to trust your Apple \
             Account: Settings › General › VPN & Device Management.",
            "Free accounts' apps last 7 days; SideInstaller can renew itself from then on.",
        ],
    );
    true
}
