//! Step 4: sign in to the user's Apple Account, as SideInstaller does.
//!
//! The sign-in itself is rust-core's `apple_session::open`, compiled here
//! unchanged (see main.rs): a saved developer token first, then a full
//! GrandSlam sign-in with two-factor, the -80046 skip for Android-ADI servers,
//! and the current-Mac identity rules. What's added is the terminal: the
//! prompts, and the order servers are tried in.
//!
//! The first time, ssboot starts from what siboot saved in the same Minis, if
//! it did: its signing key and certificate, anisette identity and sign-in. One
//! certificate then signs SideInstaller and SideStore alike, instead of a
//! second one taking a free account's last place, or making ssboot ask which
//! to revoke.
//!
//! Also here: `Account.sideconf`, the signing certificate as SideStore imports
//! it, built as rust-core's `account_config` builds it.

use std::path::{Path, PathBuf};

use base64::{prelude::BASE64_STANDARD, Engine as _};
use isideload::anisette::remote_v3::{state::AnisetteState, state_key_for};
use isideload::auth::apple_account::{TwoFactorCallbackParams, TwoFactorCallbackResponse};
use isideload::dev::certificates::DevelopmentCertificate;
use isideload::sideload::builder::MaxCertsBehavior;
use isideload::sideload::cert_identity::CertificateIdentity;
use isideload::sideload::sideloader::Sideloader;
use isideload::sideload::{SideloaderBuilder, TeamSelection};
use isideload::util::fs_storage::FsStorage;
use isideload::util::storage::SideloadingStorage;
use rootcause::Report;

use crate::apple_session;
use crate::fail::{Fail, Result};
use crate::{ui, Args};

/// Anisette servers on Apple's macOS ADI library, in the order SideInstaller's
/// Auto tries them (`AnisetteServer.macADIOrder`), then its Windows-library one.
/// Android-library servers are left out, as there: Apple refuses the token they
/// lead to (-80046) after the password and code have gone through.
pub const SERVERS: &[&str] = &[
    "https://new.ani.sidestore.io",
    "https://mac-ani.jkcoxson.com",
    "https://ani.iloader.app",
    "https://ani3.stikstore.app",
    "https://ani.jkcoxson.com",
    "https://ani2.stikstore.app",
    "https://win.ani.sidestore.io",
];

/// The name the signing certificate is made for, as Xcode names a Mac's.
const MACHINE_NAME: &str = "ssboot";

pub struct Session {
    pub sideloader: Sideloader,
    /// The developer team signing for it, as "name (ID)".
    pub team: String,
    /// isideload's store: the signing key, anisette identities, saved sign-in.
    storage: PathBuf,
    /// The anisette server signed in through. Each server family keeps its
    /// own identity, and a hand-off carries the one the token was issued to.
    anisette_url: String,
}

pub async fn sign_in(args: &Args) -> Result<Session> {
    let apple_id = match args
        .apple_id
        .clone()
        .or_else(|| std::env::var("SSBOOT_APPLE_ID").ok())
    {
        Some(id) => id,
        None => ui::ask_text(
            "sign in to your Apple Account",
            "Email",
            "your Apple Account",
            "Apple Account email: ",
        )
        .unwrap_or_default(),
    };
    let apple_id = apple_id.trim().to_string();
    if apple_id.is_empty() {
        return Err(Fail::by_user("No Apple Account given."));
    }
    let password = match std::env::var("SSBOOT_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => ui::ask_secret(
            "sign in to your Apple Account",
            "Password",
            "it goes only to Apple",
            "Password (not shown; it goes only to Apple): ",
        )
        .map_err(|e| {
            Fail::new(
                "Couldn't read the password from the terminal.",
                e.to_string(),
            )
        })?
        .unwrap_or_default(),
    };
    if password.is_empty() {
        return Err(Fail::by_user("No password given."));
    }

    let storage = args.home.join("isideload");
    if let Some(siboot) = crate::siboot_home() {
        start_from_siboot(&siboot, &args.home);
    }
    std::fs::create_dir_all(&storage).map_err(|e| {
        Fail::new(
            format!("Couldn't create {}.", storage.display()),
            e.to_string(),
        )
    })?;

    let servers = candidates(args.anisette.as_deref(), last_server(&args.home).as_deref());
    let mut last_error = String::new();
    for server in &servers {
        ui::status(&format!("signing in through {}", host_of(server)));
        match apple_session::open(
            &apple_id, &password, server, &storage, true, two_factor, "Apple ID",
        )
        .await
        {
            Ok((developer, teams)) => {
                remember_server(&args.home, server);
                let mut sideloader = SideloaderBuilder::new(developer, apple_id.clone())
                    .team_selection(TeamSelection::First)
                    .max_certs_behavior(MaxCertsBehavior::Prompt(Box::new(
                        choose_certificates_to_revoke,
                    )))
                    .storage(Box::new(FsStorage::new(storage.clone())))
                    .machine_name(MACHINE_NAME.to_string())
                    .build();
                if let Some(first) = teams.into_iter().next() {
                    sideloader.set_team(first);
                }
                let team = sideloader.get_team().await.map_err(|e| {
                    Fail::new(
                        "Signed in, but Apple didn't list a developer team.",
                        crate::error_text::report_text(&e),
                    )
                })?;
                let team = format!(
                    "{} ({})",
                    team.name.as_deref().unwrap_or("unnamed"),
                    team.team_id
                );
                ui::ok(&format!("signed in — team {team}"));
                return Ok(Session {
                    sideloader,
                    team,
                    storage,
                    anisette_url: server.clone(),
                });
            }
            Err(e) => {
                if let Some(advice) = stops_sign_in(&e) {
                    let mut f = Fail::new(advice, e);
                    f.by_user = advice == CANCELLED;
                    return Err(f);
                }
                ui::warn(&format!(
                    "{} didn't work: {}",
                    host_of(server),
                    first_line(&e)
                ));
                last_error = e;
            }
        }
    }
    Err(Fail::new(
        format!(
            "Couldn't sign in through any of {} anisette servers. Try again later.",
            servers.len()
        ),
        last_error,
    ))
}

/// `pick` alone when given; otherwise the last server that worked, then the
/// rest of `SERVERS`.
fn candidates(pick: Option<&str>, last: Option<&str>) -> Vec<String> {
    if let Some(pick) = pick {
        return vec![pick.trim().trim_end_matches('/').to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    for s in last.into_iter().chain(SERVERS.iter().copied()) {
        let s = s.trim().trim_end_matches('/').to_string();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

fn last_server_path(home: &Path) -> PathBuf {
    home.join("anisette-server")
}

fn last_server(home: &Path) -> Option<String> {
    std::fs::read_to_string(last_server_path(home))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn remember_server(home: &Path, server: &str) {
    let _ = std::fs::write(last_server_path(home), server);
}

/// When ssboot has no isideload store yet and siboot's folder (`siboot`) has
/// one, starts from a copy of it, and of the anisette server that last
/// worked. Returns how many files were copied, or None when nothing was.
fn start_from_siboot(siboot: &Path, home: &Path) -> Option<usize> {
    let ours = home.join("isideload");
    let theirs = siboot.join("isideload");
    if siboot == home || ours.exists() || !theirs.is_dir() {
        return None;
    }
    match copy_dir(&theirs, &ours) {
        Ok(0) => {
            let _ = std::fs::remove_dir_all(&ours);
            None
        }
        Ok(files) => {
            if !last_server_path(home).exists() {
                if let Some(server) = last_server(siboot) {
                    remember_server(home, &server);
                }
            }
            ui::ok("starting from the signing certificate and sign-in siboot saved");
            ui::detail(&format!("copied {files} files from {}", theirs.display()));
            Some(files)
        }
        Err(e) => {
            // Half a store is worse than none: isideload starts a fresh one.
            let _ = std::fs::remove_dir_all(&ours);
            ui::note("Couldn't copy what siboot saved, so ssboot starts afresh.");
            ui::detail(&e.to_string());
            None
        }
    }
}

/// Copies the folder `from` to `to`, which mustn't exist yet, keeping each
/// file's permissions: the store holds private keys. Returns the files copied.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut files = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            files += copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
            files += 1;
        }
    }
    Ok(files)
}

/// `Account.sideconf`: the JSON SideStore imports on launch and then deletes
/// (`LaunchViewController.detectAndImportAccountFile`), as rust-core's
/// `account_config` builds it. SideStore can sign only with a certificate
/// whose private key it has; without this it makes its own the first time you
/// sign in to it.
///
/// The certificate goes in as a PKCS#12 archive whose password is its machine
/// ID, as AltStore-family apps expect, with the anisette identity this session
/// signed in with. No password: SideStore asks for it anyway. Only looks up the
/// certificate signing made, never creates or revokes one.
pub async fn account_config(session: &mut Session) -> std::result::Result<String, String> {
    let storage = FsStorage::new(session.storage.clone());
    let key = state_key_for(&storage, &session.anisette_url);
    let raw = storage
        .retrieve_data(&key)
        .map_err(|e| format!("could not read the anisette state: {e}"))?
        .ok_or("no anisette state stored yet")?;
    let state: AnisetteState =
        plist::from_bytes(&raw).map_err(|e| format!("could not parse the anisette state: {e}"))?;
    let adi_pb = state
        .adi_pb
        .clone()
        .ok_or("the anisette identity was never provisioned")?;

    let team = session
        .sideloader
        .get_team()
        .await
        .map_err(|e| format!("get_team: {}", crate::error_text::report_text(&e)))?;
    let email = session.sideloader.get_email().to_string();
    let identity = CertificateIdentity::retrieve_existing(
        &email,
        session.sideloader.get_dev_session(),
        &team,
        &storage,
    )
    .await
    .map_err(|e| format!("certificate lookup failed: {}", crate::error_text::report_text(&e)))?
    .ok_or("no certificate for ssboot's signing key on this Apple Account yet")?;
    let password = identity.machine_id.clone();
    let p12 = identity
        .as_p12(&password)
        .await
        .map_err(|e| format!("failed to build the PKCS#12 archive: {e}"))?;
    Ok(serde_json::json!({
        "version": "2.0",
        "email": email,
        "certificateData": BASE64_STANDARD.encode(&p12),
        "certType": "encrypted",
        "certificatePassword": password,
        "anisetteIdentifier": BASE64_STANDARD.encode(state.keychain_identifier),
        "anisetteAdiBlob": BASE64_STANDARD.encode(adi_pb),
    })
    .to_string())
}

/// The user said no to every way of getting the code: not a problem to log.
const CANCELLED: &str = "Sign-in cancelled.";

/// Advice when another server can't help: the same account would meet the
/// same answer, and retrying risks Apple locking it.
fn stops_sign_in(error: &str) -> Option<&'static str> {
    // Codes from SideInstaller's `Engine.credentialErrorCodes`.
    if ["-20101", "-20970", "-22406"]
        .iter()
        .any(|c| error.contains(c))
    {
        return Some(
            "Apple rejected the Apple Account or password. Check them and run ssboot again. If \
             you're sure they're right, Apple may be limiting sign-ins: wait a while first.",
        );
    }
    if error.contains("429") || error.to_ascii_lowercase().contains("too many") {
        return Some(
            "Apple is limiting sign-in attempts for now. Wait a while, then run ssboot again.",
        );
    }
    if error.contains("No 2FA") {
        return Some(CANCELLED);
    }
    None
}

fn host_of(url: &str) -> &str {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

/// isideload's two-factor callback: the prompt runs on a blocking thread, since
/// it waits on the terminal.
fn two_factor(
    params: TwoFactorCallbackParams,
) -> impl std::future::Future<Output = std::result::Result<TwoFactorCallbackResponse, Report>> + Send
{
    let prompt = Prompt::from(&params);
    async move {
        let answer = tokio::task::spawn_blocking(move || ask_two_factor(&prompt))
            .await
            .unwrap_or(TwoFactorCallbackResponse::Abort);
        Ok(answer)
    }
}

/// What Apple is waiting for, owned so it can cross to the prompt's thread.
struct Prompt {
    /// Where the pending code went.
    method: Method,
    selected: Option<u32>,
    last_error: Option<String>,
    numbers: Vec<(u32, String, String)>,
}

#[derive(PartialEq, Eq, Debug)]
enum Method {
    Devices,
    Sms,
    Voice,
    /// Nothing is pending: the user picks.
    Choose,
}

impl From<&TwoFactorCallbackParams> for Prompt {
    fn from(p: &TwoFactorCallbackParams) -> Self {
        let method = if p.unknown {
            Method::Choose
        } else if p.voice {
            Method::Voice
        } else if p.sms {
            Method::Sms
        } else {
            Method::Devices
        };
        Self {
            method,
            selected: p.selected_number_id,
            last_error: p.last_error.clone(),
            numbers: p
                .numbers
                .iter()
                .map(|n| (n.id, n.number_with_dial_code.clone(), n.push_mode.clone()))
                .collect(),
        }
    }
}

fn ask_two_factor(prompt: &Prompt) -> TwoFactorCallbackResponse {
    if ui::is_live() {
        if let Some(answer) = ask_two_factor_live(prompt) {
            return answer;
        }
    }
    ask_two_factor_plain(prompt)
}

impl Prompt {
    fn number(&self, id: Option<u32>) -> String {
        self.numbers
            .iter()
            .find(|(n, _, _)| Some(*n) == id)
            .map(|(_, number, _)| number.clone())
            .unwrap_or_else(|| "your trusted number".into())
    }

    /// The other ways to get a code, as a menu: each trusted number (texted,
    /// or called), the devices, again, or not at all.
    fn ways(&self) -> Vec<(String, TwoFactorCallbackResponse)> {
        let mut ways = Vec::new();
        for (id, number, mode) in &self.numbers {
            // "voice": the number takes calls only.
            if mode != "voice" {
                ways.push((
                    format!("Text {number}"),
                    TwoFactorCallbackResponse::SendSms(*id),
                ));
            }
            ways.push((
                format!("Call {number}"),
                TwoFactorCallbackResponse::CallNumber(*id),
            ));
        }
        ways.push((
            "Send it to your Apple devices".into(),
            TwoFactorCallbackResponse::SendToDevices,
        ));
        if self.method != Method::Choose {
            ways.push((
                "Send the same again".into(),
                TwoFactorCallbackResponse::ResendCode,
            ));
        }
        ways.push((
            "Cancel the sign-in".into(),
            TwoFactorCallbackResponse::Abort,
        ));
        ways
    }
}

/// The code in boxes, and a menu of other ways to get it. None when the
/// screen can't read keys after all.
fn ask_two_factor_live(prompt: &Prompt) -> Option<TwoFactorCallbackResponse> {
    let sent = match prompt.method {
        Method::Devices => Some("sent to your Apple devices".to_string()),
        Method::Sms => Some(format!("texted to {}", prompt.number(prompt.selected))),
        Method::Voice => Some(format!(
            "Apple is calling {}",
            prompt.number(prompt.selected)
        )),
        Method::Choose => None,
    };
    let mut warning = prompt.last_error.as_ref().map(|e| format!("Apple: {e}"));
    loop {
        if let Some(sent) = &sent {
            let mut hints = Vec::new();
            if let Some(warning) = warning.take() {
                hints.push(ui::Hint::Warning(warning));
            }
            hints.push(ui::Hint::Plain(sent.clone()));
            hints.push(ui::Hint::Plain("o: other ways · esc: cancel".into()));
            match ui::ask_code_or_more("enter the code Apple sent", "Code", &hints)? {
                ui::CodeAnswer::Code(code) => {
                    return Some(TwoFactorCallbackResponse::SubmitCode(code))
                }
                ui::CodeAnswer::Cancel => return Some(TwoFactorCallbackResponse::Abort),
                ui::CodeAnswer::More => {}
            }
        }
        let ways = prompt.ways();
        let labels: Vec<String> = ways.iter().map(|(label, _)| label.clone()).collect();
        match ui::choose("how should Apple send the code?", &labels) {
            Some(i) => return ways.into_iter().nth(i).map(|(_, answer)| answer),
            // Back to the boxes, when a code is on its way.
            None if sent.is_some() => continue,
            None => return Some(TwoFactorCallbackResponse::Abort),
        }
    }
}

fn ask_two_factor_plain(prompt: &Prompt) -> TwoFactorCallbackResponse {
    println!();
    if let Some(error) = &prompt.last_error {
        ui::warn(&format!("Apple: {error}"));
    }
    match prompt.method {
        Method::Devices => {
            ui::status("Apple sent a verification code to your other Apple devices.")
        }
        Method::Sms => ui::status(&format!(
            "Apple texted a verification code to {}.",
            prompt.number(prompt.selected)
        )),
        Method::Voice => ui::status(&format!(
            "Apple is calling {} with a verification code.",
            prompt.number(prompt.selected)
        )),
        Method::Choose => ui::status("Choose how Apple should send the verification code."),
    }
    for (id, number, mode) in &prompt.numbers {
        let how = if mode == "voice" { " (calls only)" } else { "" };
        ui::status(&format!("  {id}: {number}{how}"));
    }
    ui::status("Type the code, or: t N to text number N, c N to call it, d for your devices,");
    ui::status("r to send it again, q to give up.");
    loop {
        let Some(line) = ui::ask("Code: ") else {
            return TwoFactorCallbackResponse::Abort;
        };
        match parse_two_factor(&line) {
            Some(answer) => return answer,
            None if line.is_empty() => continue,
            None => ui::warn("That isn't a 6-digit code or one of t N, c N, d, r, q."),
        }
    }
}

fn parse_two_factor(line: &str) -> Option<TwoFactorCallbackResponse> {
    let line = line.trim();
    let digits: String = line
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if digits.len() == 6 && digits.chars().all(|c| c.is_ascii_digit()) {
        return Some(TwoFactorCallbackResponse::SubmitCode(digits));
    }
    let mut words = line.split_whitespace();
    let (Some(verb), id) = (
        words.next(),
        words.next().and_then(|n| n.parse::<u32>().ok()),
    ) else {
        return None;
    };
    match (verb.to_ascii_lowercase().as_str(), id) {
        ("t" | "text" | "sms", Some(id)) => Some(TwoFactorCallbackResponse::SendSms(id)),
        ("c" | "call" | "voice", Some(id)) => Some(TwoFactorCallbackResponse::CallNumber(id)),
        ("d" | "devices", None) => Some(TwoFactorCallbackResponse::SendToDevices),
        ("r" | "resend", None) => Some(TwoFactorCallbackResponse::ResendCode),
        ("q" | "quit", None) => Some(TwoFactorCallbackResponse::Abort),
        _ => None,
    }
}

/// isideload asks this when the account already has as many development
/// certificates as Apple allows. Revoking one stops every app signed with it
/// (on any device) from opening, so nothing is revoked without the user typing
/// which.
#[allow(clippy::ptr_arg)] // isideload's `MaxCertsBehavior::Prompt` takes a `&Vec`
fn choose_certificates_to_revoke(certs: &Vec<DevelopmentCertificate>) -> Option<Vec<String>> {
    tokio::task::block_in_place(|| {
        if !ui::is_live() {
            return choose_certificates_plain(certs);
        }
        ui::warn(
            "This Apple Account already has as many signing certificates as Apple allows, so one \
             must be revoked to make ssboot's. Apps signed with a revoked certificate stop \
             opening, on every device, until they're signed again. This can't be undone.",
        );
        let mut labels: Vec<String> = certs
            .iter()
            .map(|cert| {
                format!(
                    "{} · \"{}\" · {}",
                    cert.name.as_deref().unwrap_or("certificate"),
                    cert.machine_name.as_deref().unwrap_or("?"),
                    cert.serial_number.as_deref().unwrap_or("?"),
                )
            })
            .collect();
        labels.push("None: stop here".into());
        let index = ui::choose("revoke which certificate?", &labels)?;
        let serial = certs.get(index)?.serial_number.clone()?;
        if !ui::confirm("revoke it? this can't be undone", false) {
            return None;
        }
        ui::status(&format!("revoking certificate {serial}"));
        Some(vec![serial])
    })
}

fn choose_certificates_plain(certs: &[DevelopmentCertificate]) -> Option<Vec<String>> {
    println!();
    ui::warn("This Apple Account already has as many signing certificates as Apple allows,");
    ui::warn("so one must be revoked to make ssboot's. Apps signed with a revoked certificate");
    ui::warn("stop opening, on every device, until they're signed again. This can't be undone.");
    for (i, cert) in certs.iter().enumerate() {
        ui::status(&format!(
            "  {}: {} — made on \"{}\", serial {}",
            i + 1,
            cert.name.as_deref().unwrap_or("certificate"),
            cert.machine_name.as_deref().unwrap_or("?"),
            cert.serial_number.as_deref().unwrap_or("?"),
        ));
    }
    let line = ui::ask("Number to revoke (empty to stop): ")?;
    let index = line.trim().parse::<usize>().ok()?.checked_sub(1)?;
    let serial = certs.get(index)?.serial_number.clone()?;
    ui::status(&format!("revoking certificate {serial}"));
    Some(vec![serial])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is(answer: Option<TwoFactorCallbackResponse>, expected: &str) -> bool {
        format!("{answer:?}") == expected
    }

    #[test]
    fn two_factor_answers_parse() {
        assert!(is(
            parse_two_factor("123456"),
            "Some(SubmitCode(\"123456\"))"
        ));
        assert!(is(
            parse_two_factor(" 123 456 "),
            "Some(SubmitCode(\"123456\"))"
        ));
        assert!(is(
            parse_two_factor("123-456"),
            "Some(SubmitCode(\"123456\"))"
        ));
        assert!(is(parse_two_factor("t 2"), "Some(SendSms(2))"));
        assert!(is(parse_two_factor("call 1"), "Some(CallNumber(1))"));
        assert!(is(parse_two_factor("d"), "Some(SendToDevices)"));
        assert!(is(parse_two_factor("r"), "Some(ResendCode)"));
        assert!(is(parse_two_factor("q"), "Some(Abort)"));
        for bad in ["12345", "1234567", "t", "t x", "d 1", "hello", ""] {
            assert!(parse_two_factor(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn servers_go_last_good_first_and_a_pick_alone() {
        let all = candidates(None, Some("https://ani.iloader.app/"));
        assert_eq!(all[0], "https://ani.iloader.app");
        assert_eq!(
            all.len(),
            SERVERS.len(),
            "no duplicate of the last good one"
        );
        assert_eq!(candidates(None, None)[0], SERVERS[0]);
        assert_eq!(
            candidates(
                Some("https://example.org/"),
                Some("https://ani.iloader.app")
            ),
            ["https://example.org"]
        );
    }

    #[test]
    fn a_first_run_starts_from_what_siboot_saved_once() {
        let root = std::env::temp_dir().join(format!("ssboot-start-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let siboot = root.join(".siboot");
        let home = root.join(".ssboot");
        std::fs::create_dir_all(siboot.join("isideload/keys")).unwrap();
        std::fs::write(siboot.join("isideload/keys/key.pem"), b"key").unwrap();
        std::fs::write(siboot.join("isideload/state"), b"state").unwrap();
        std::fs::write(siboot.join("anisette-server"), "https://ani.iloader.app\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key = siboot.join("isideload/keys/key.pem");
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        assert_eq!(start_from_siboot(&siboot, &home), Some(2));
        assert_eq!(std::fs::read(home.join("isideload/keys/key.pem")).unwrap(), b"key");
        assert_eq!(last_server(&home).as_deref(), Some("https://ani.iloader.app"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(home.join("isideload/keys/key.pem"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "a private key stays private");
        }

        // Once ssboot has its own, it keeps it.
        std::fs::write(siboot.join("isideload/state"), b"newer").unwrap();
        assert_eq!(start_from_siboot(&siboot, &home), None);
        assert_eq!(std::fs::read(home.join("isideload/state")).unwrap(), b"state");

        // Nothing saved, nothing copied; and never from itself.
        assert_eq!(start_from_siboot(&root.join("none"), &root.join("other")), None);
        assert!(!root.join("other/isideload").exists());
        assert_eq!(start_from_siboot(&siboot, &siboot), None);
    }

    #[test]
    fn credential_errors_stop_and_server_errors_move_on() {
        assert!(stops_sign_in("login failed: -22406 Enter the correct password").is_some());
        assert!(stops_sign_in("login failed: HTTP 429").is_some());
        assert!(stops_sign_in("login failed: No 2FA code provided, aborting").is_some());
        assert!(stops_sign_in("anisette provider: error sending request").is_none());
        assert!(stops_sign_in(apple_session::ANDROID_ADI_SKIP).is_none());
    }

    /// The certificate prompt blocks in place, and isideload calls it from the
    /// pipeline, which `main` runs with `Runtime::block_on` on a two-worker
    /// runtime, not on a worker. That combination has to be allowed.
    #[test]
    fn the_revoke_prompt_may_block_where_the_pipeline_runs() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let answer = runtime.block_on(async { tokio::task::block_in_place(|| 7) });
        assert_eq!(answer, 7);
    }
}
