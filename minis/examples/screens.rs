//! ssboot's live screen through a made-up run, for working on the UI without
//! an iPhone. Nothing here talks to a device or to Apple:
//!
//! ```sh
//! cargo run --example screens                  # an install, pairing in Minis
//! cargo run --example screens -- rootshell     # taking rootshell's pairing
//! cargo run --example screens -- stop          # one that stops at Connect
//! cargo run --example screens -- old-minis     # in an Open Minis before 1.16
//! cargo run --example screens -- teams         # an Apple Account on two teams
//! cargo run --example screens -- saved         # two saved Apple Accounts
//! cargo run --example screens -- saved run-out # whose saved sign-in has run out
//! cargo run --example screens -- moved         # the pairing service on 55021
//! cargo run --example screens -- nowhere       # and on no port at all
//! cargo run --example screens -- vpn           # another VPN on, then LocalDevVPN
//! cargo run --example screens -- blocked       # GitHub blocked, SideStore kept
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

#[path = "../src/saved.rs"]
mod saved;
#[path = "../src/team.rs"]
mod team;
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
    let port = if has("moved") {
        Port::Moved
    } else if has("nowhere") {
        Port::Nowhere
    } else {
        Port::Usual
    };
    let github = if has("vpn") {
        Github::OtherVpn
    } else if has("blocked") {
        Github::Blocked
    } else {
        Github::Answers
    };
    let remembered = match (has("saved"), has("run-out")) {
        (true, false) => Some(Remembered::Lasting),
        (true, true) => Some(Remembered::RunOut),
        _ => None,
    };
    let code = if has("old-minis") {
        old_minis();
        1
    } else if run(devmode, stop, has("rootshell"), has("teams"), port, github, remembered) {
        0
    } else {
        1
    };
    if (stop || port == Port::Nowhere) && ui::confirm("copy the log to the clipboard?", true) {
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

/// The stop in an Open Minis older than 1.16 (preflight.rs), with no log to
/// offer after it.
fn old_minis() {
    ui::header("0.3.2", "Installs SideStore on this iPhone.");
    ui::plan(
        &["Check", "Pairing", "Connect", "Apple ID", "Sign", "Install", "Handover"],
        7,
    );
    ui::step(1, "Checking LocalDevVPN, this iPhone and the internet");
    ui::activity("checking this Open Minis");
    wait(400);
    ui::link("Update Open Minis", "itms-apps://apps.apple.com/app/id6759188481");
    ui::failure(
        "This Open Minis is older than 1.16, and ssboot needs Open Minis 1.16 or later: it's the \
         first that can pair this iPhone. Update Open Minis in the App Store, open it again and \
         run ssboot.",
        "",
    );
}

/// Where step 1 finds the iPhone's pairing service (preflight.rs).
#[derive(Clone, Copy, PartialEq)]
enum Port {
    /// On 49152, as nearly always.
    Usual,
    /// 49152 refuses, and the look through the other ports finds it.
    Moved,
    /// Nothing answers as it on any port.
    Nowhere,
}

/// How GitHub answers step 1 (download.rs, preflight.rs).
#[derive(Clone, Copy, PartialEq)]
enum Github {
    /// Through LocalDevVPN, as it does where GitHub isn't blocked.
    Answers,
    /// Through another VPN, on to reach it where it's blocked: SideStore
    /// downloads, nothing answers at 10.7.0.1, and ssboot asks for LocalDevVPN.
    OtherVpn,
    /// Not at all, so ssboot installs the SideStore an earlier run kept.
    Blocked,
}

/// Step 1's download of SideStore, before the iPhone is dialed.
fn get_sidestore(github: Github) {
    ui::activity("asking GitHub for SideStore's latest nightly");
    wait(600);
    if github == Github::Blocked {
        ui::warn(
            "Couldn't reach GitHub, so ssboot installs the SideStore nightly downloaded on an \
             earlier run.",
        );
        return;
    }
    ui::status("downloading SideStore's nightly");
    let total: u64 = 30_681_634;
    for i in 0..=50u64 {
        ui::progress("downloaded", total * i / 50, total);
        wait(45);
    }
    ui::progress_end();
    ui::ok("downloaded SideStore's nightly (29.3 MB)");
}

/// Step 1's dial of the pairing service, and the look through the iPhone's
/// other ports that follows a refusal.
fn reach(port: Port, github: Github) -> bool {
    ui::activity("reaching this iPhone at 10.7.0.1:49152");
    wait(500);
    if github == Github::OtherVpn {
        wait(1500);
        ui::detail("nothing answers at 10.7.0.1:49152: connect timed out after 8 s");
        ui::status("connect LocalDevVPN when you're ready");
        if !ui::get_ready(
            "Connect LocalDevVPN",
            &[
                "Open LocalDevVPN and connect it. iOS runs one VPN at a time, so it takes the \
                 place of any other: ssboot has SideStore already, and doesn't need GitHub \
                 again.",
                "Come back to Minis once LocalDevVPN shows Connected.",
            ],
            &[],
            "Continue",
        ) {
            ui::failure("Stopped before LocalDevVPN was connected.", "");
            return false;
        }
        ui::activity("reaching this iPhone at 10.7.0.1:49152");
        wait(500);
    }
    if port == Port::Usual {
        ui::ok("this iPhone answers on 10.7.0.1:49152 (RPPairing, wire protocol 26)");
        return true;
    }
    ui::detail(
        "no pairing service at 10.7.0.1:49152: Connection refused (os error 111) \
         (ConnectionRefused)",
    );
    ui::status("looking for this iPhone's pairing service on its other ports");
    // 55021 is 5869 of the 16382 ports in; nowhere knocks on all of them.
    let last = if port == Port::Moved { 35 } else { 100 };
    for percent in 0..=last {
        ui::percent("ports checked", percent);
        wait(40);
    }
    ui::progress_end();
    if port == Port::Nowhere {
        ui::failure(
            "The iPhone refused the connection at 10.7.0.1:49152, and no other port on it answers \
             as its pairing service. Turn on Developer Mode (Settings › Privacy & Security › \
             Developer Mode), and check Wi-Fi is on. If both are on, restart the iPhone, connect \
             LocalDevVPN again and run ssboot again.",
            "Connection refused (os error 111) (ConnectionRefused); no other port answered as it \
             (16382 knocked on in 4.1 s; took the connection: 54666)",
        );
        return false;
    }
    ui::detail("found it on port 55021 after 5869 of 16382 ports, in 1.4 s");
    ui::ok("this iPhone answers on 10.7.0.1:55021 (RPPairing, wire protocol 26)");
    true
}

fn run(
    devmode: bool,
    stop: bool,
    rootshell: bool,
    teams: bool,
    port: Port,
    github: Github,
    remembered: Option<Remembered>,
) -> bool {
    let steps = [
        "Check", "Pairing", "Connect", "Apple ID", "Sign", "Install", "Handover",
    ];
    if devmode {
        ui::header("0.3.2", "Shows Developer Mode, or turns it on.");
        ui::plan(&["Check", "Pairing", "Connect", "Dev Mode"], 4);
    } else {
        ui::header("0.3.2", "Installs SideStore on this iPhone.");
        ui::plan(&steps, 7);
    }

    if devmode {
        ui::step(1, "Checking LocalDevVPN and this iPhone");
    } else {
        ui::step(1, "Checking the internet, LocalDevVPN and this iPhone");
        ui::activity("reaching Apple's sign-in server");
        wait(600);
        ui::ok("Apple's sign-in server answers");
        get_sidestore(github);
    }
    if !reach(port, if devmode { Github::Answers } else { github }) {
        return false;
    }
    ui::done(if devmode { "the iPhone answers" } else { "SideStore's nightly · the iPhone answers" });

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
    let picked = match remembered {
        Some(_) => saved_account(),
        None => ask_email().map(|_| Picked::Typed),
    };
    let Some(picked) = picked else {
        ui::failure("No Apple Account given.", "");
        return false;
    };
    let with_password = match picked {
        Picked::Saved if remembered == Some(Remembered::Lasting) => false,
        Picked::Saved => {
            ui::note(saved::RUN_OUT);
            true
        }
        Picked::Typed => true,
    };
    if with_password {
        if !password_and_code() {
            return false;
        }
    } else {
        ui::status("signing in through new.ani.sidestore.io");
        wait(1400);
        ui::detail("used the saved sign-in: Apple's sign-in lasts 341d 7h more");
    }
    let (name, id, kind, first) = if teams {
        // In Apple's order, which account.rs keeps: here the free team first.
        let listed = [
            ("Frizzle", "ABCDE12345", team::Kind::Free),
            ("Frizzle", "FGHIJ67890", team::Kind::Paid),
        ];
        ui::status("reading your teams");
        wait(700);
        let labels: Vec<String> = listed
            .iter()
            .map(|(name, id, kind)| team::label(Some(name), id, *kind))
            .collect();
        ui::status(&team::found(listed.len()));
        ui::note(team::NOTE);
        let Some(i) = ui::choose(team::QUESTION, &labels) else {
            ui::failure("No developer team picked.", "");
            return false;
        };
        let (name, id, kind) = listed[i];
        (name, id, kind, i == 0)
    } else {
        ("Frizzle", "ABCDE12345", team::Kind::Unknown, true)
    };
    let summary = team::summary(Some(name), id, kind);
    ui::ok(&if with_password {
        format!("signed in — {summary}")
    } else {
        format!("signed in without the password — {summary}")
    });
    if picked == Picked::Typed {
        ui::note(saved::SAVED);
    }
    ui::done(&summary);
    sign_and_install(id, kind, first)
}

fn ask_email() -> Option<String> {
    ui::ask_text(
        "sign in to your Apple Account",
        "Email",
        "your Apple Account",
        "Apple Account email: ",
    )
}

/// What the sign-in has saved, in `-- saved`.
#[derive(Clone, Copy, PartialEq)]
enum Remembered {
    /// Two Apple Accounts, and the first one's sign-in still works.
    Lasting,
    /// The same, but Apple wants the password again.
    RunOut,
}

#[derive(Clone, Copy, PartialEq)]
enum Picked {
    Saved,
    Typed,
}

/// The menu of saved Apple Accounts, as account.rs's `which_account` asks it:
/// one picked, another typed, or one forgotten and the menu again.
fn saved_account() -> Option<Picked> {
    let mut accounts = vec![
        "frizzle@icloud.com".to_string(),
        "frizzle.work@example.com".to_string(),
    ];
    loop {
        if accounts.is_empty() {
            return ask_email().map(|_| Picked::Typed);
        }
        let index = ui::choose(saved::QUESTION, &saved::options(&accounts))?;
        match saved::picked(index, accounts.len()) {
            saved::Pick::Account(_) => return Some(Picked::Saved),
            saved::Pick::Another => {
                if ask_email().is_some() {
                    return Some(Picked::Typed);
                }
            }
            saved::Pick::Forget => {
                let index = match accounts.as_slice() {
                    [_] => Some(0),
                    _ => ui::choose(saved::FORGET_WHICH, &accounts),
                };
                if let Some(i) = index {
                    if ui::confirm(&format!("forget {}?", accounts[i]), false) {
                        let email = accounts.remove(i);
                        ui::ok(&format!("forgot {email} and its saved sign-in"));
                    }
                }
            }
        }
    }
}

/// The password, a server that doesn't answer, and Apple's code. False once
/// the run has stopped.
fn password_and_code() -> bool {
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
    true
}

/// Steps 5 to 7, signed by team `id`. SideStore picks its own team when it's
/// signed in to, so the finish box goes by `kind` only.
fn sign_and_install(id: &str, kind: team::Kind, _first: bool) -> bool {
    ui::step(5, "Signing SideStore for this iPhone");
    ui::activity("checking it's SideStore");
    wait(600);
    ui::ok("it's SideStore 0.7.0-20261027.1507");
    ui::status("signing; this takes a while here");
    wait(4200);
    ui::ok(&format!("signed SideStore as com.SideStore.SideStore.{id}"));
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
    ui::ok(&format!("SideStore installed (com.SideStore.SideStore.{id})"));
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
            team::lasts_sidestore(kind),
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
