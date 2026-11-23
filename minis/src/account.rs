//! Step 4: sign in to the user's Apple Account, as SideInstaller does.
//!
//! The sign-in itself is rust-core's `apple_session::open`, compiled here
//! unchanged (see main.rs): a saved developer token first, then a full
//! GrandSlam sign-in with two-factor, the -80046 skip for Android-ADI servers,
//! and the current-Mac identity rules. What's added is the terminal: the
//! prompts, the order servers are tried in, and which team signs when Apple
//! lists more than one (team.rs).
//!
//! The first time, ssboot starts from what siboot saved in the same Minis, if
//! it did: its signing key and certificate, anisette identity and sign-in. One
//! certificate then signs SideInstaller and SideStore alike, instead of a
//! second one taking a free account's last place, or making ssboot ask which
//! to revoke.
//!
//! Also here: `Account.sideconf`, the signing certificate as SideStore imports
//! it, built as rust-core's `account_config` builds it.
//!
//! Since 0.3.5 (siboot 0.20.0) ssboot also remembers the Apple Accounts that
//! signed in (saved.rs) and offers them, and tries the token saved with one
//! before it asks for the password, which then only comes when Apple wants it
//! again.
//! `open` takes the password up front, so that first try is ssboot's own: the
//! same two tries of the token `open` makes, without the login it falls back
//! to.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{prelude::BASE64_STANDARD, Engine as _};
use isideload::anisette::remote_v3::{state::AnisetteState, state_key_for};
use isideload::anisette::{
    remote_v3::RemoteV3AnisetteProvider, AnisetteDataGenerator, AnisetteProvider,
};
use isideload::auth::apple_account::{
    AppToken, AppleAccount, TwoFactorCallbackParams, TwoFactorCallbackResponse,
};
use isideload::auth::grandslam::GrandSlam;
use isideload::dev::certificates::DevelopmentCertificate;
use isideload::dev::developer_session::DeveloperSession;
use isideload::dev::device_type::{dev_url, DeveloperDeviceType};
use isideload::dev::teams::{DeveloperTeam, TeamsApi};
use isideload::sideload::builder::MaxCertsBehavior;
use isideload::sideload::cert_identity::CertificateIdentity;
use isideload::sideload::sideloader::Sideloader;
use isideload::sideload::{SideloaderBuilder, TeamSelection};
use isideload::util::fs_storage::FsStorage;
use isideload::util::storage::SideloadingStorage;
use rootcause::Report;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::apple_session;
use crate::error_text::report_text;
use crate::fail::{Fail, Result};
use crate::saved;
use crate::team::{self, Kind};
use crate::{ui, Args};

/// Anisette servers on Apple's macOS ADI library, in the order SideInstaller's
/// Auto tries them: lillq's pool (`POOL`), then the others
/// (`AnisetteServer.macADIOrder`), then its Windows-library one. Android-library
/// servers are left out, as there: Apple refuses the token they lead to
/// (-80046) after the password and code have gone through.
pub const SERVERS: &[&str] = &[
    POOL,
    "https://new.ani.sidestore.io",
    "https://mac-ani.jkcoxson.com",
    "https://ani.iloader.app",
    "https://ani3.stikstore.app",
    "https://ani.jkcoxson.com",
    "https://ani2.stikstore.app",
    "https://win.ani.sidestore.io",
];

/// anisette.lillq.me, a pool of servers that checks hourly which clients Apple
/// lets through and hands out one that passes (SideInstaller's
/// `ADILibrary.pool`), first since 0.20.1. Its anisette identities work on no
/// other server, and theirs not on it, so it's tried after the servers that
/// share the last good one's identity: a saved token only works with the
/// identity it was issued to.
const POOL: &str = "https://anisette.lillq.me";

/// The name the signing certificate is made for, as Xcode names a Mac's.
const MACHINE_NAME: &str = "ssboot";

pub struct Session {
    pub sideloader: Sideloader,
    /// The developer team signing for it, as the step's summary: "paid team
    /// Frizzle (ABCDE12345)".
    pub team: String,
    /// isideload's store: the signing key, anisette identities, saved sign-in.
    storage: PathBuf,
    /// The anisette server signed in through. Each server family keeps its
    /// own identity, and a hand-off carries the one the token was issued to.
    anisette_url: String,
    /// Free or paid, when Apple listed more than one team to pick from.
    pub kind: Kind,
    /// Whether it's the first team Apple lists, the one SideInstaller renews
    /// itself with. SideStore picks its own team when it's signed in to, so
    /// ssboot's finish box goes by `kind` only.
    #[allow(dead_code)]
    pub first: bool,
}

pub async fn sign_in(args: &Args) -> Result<Session> {
    let storage = args.home.join("isideload");
    // Before the folder exists: siboot's store is only taken into none.
    if let Some(siboot) = crate::siboot_home() {
        start_from_siboot(&siboot, &args.home);
    }
    std::fs::create_dir_all(&storage).map_err(|e| {
        Fail::new(
            format!("Couldn't create {}.", storage.display()),
            e.to_string(),
        )
    })?;
    // Before this sign-in saves a token, so an account new here is said to
    // be kept.
    start_list(&args.home, &storage);
    let apple_id = match args
        .apple_id
        .clone()
        .or_else(|| std::env::var("SSBOOT_APPLE_ID").ok())
    {
        Some(id) => id,
        None => which_account(&args.home, &storage).unwrap_or_default(),
    };
    // Apple Account names aren't case-sensitive, but GrandSlam's SRP proof
    // hashes the name exactly as sent, so one typed with a capital (a phone
    // keyboard may add one) can fail as a wrong password. Lowercase, it also
    // names one storage folder per account, however it was typed.
    let typed = apple_id.trim().to_string();
    let apple_id = typed.to_lowercase();
    if apple_id.is_empty() {
        return Err(Fail::by_user("No Apple Account given."));
    }
    adopt_folder_typed_as(&storage, &typed, &apple_id);

    let servers = candidates(args.anisette.as_deref(), last_server(&args.home).as_deref());
    let password = match std::env::var("SSBOOT_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            // A sign-in saved with the account first: while Apple takes it,
            // there's no password to ask for.
            match saved_token(&storage, &apple_id, unix_now()) {
                Saved::Usable(token) => match with_token(&token, &servers, &storage).await? {
                    ByToken::Opened(server, developer, teams) => {
                        let how = Signed::WithoutPassword;
                        return signed_in(
                            args, &storage, &apple_id, &server, developer, teams, how,
                        )
                        .await;
                    }
                    ByToken::Refused => ui::note(saved::RUN_OUT),
                },
                Saved::RunOut => ui::note(saved::RUN_OUT),
                Saved::None => {}
            }
            ask_password()?
        }
    };

    let mut last_error = String::new();
    for server in &servers {
        ui::status(&format!("signing in through {}", host_of(server)));
        match apple_session::open(
            &apple_id, &password, server, &storage, true, two_factor, "Apple ID",
        )
        .await
        {
            Ok((developer, teams)) => {
                let how = Signed::WithPassword;
                return signed_in(args, &storage, &apple_id, server, developer, teams, how).await;
            }
            Err(e) => {
                if let Some(advice) = stops_sign_in(&e) {
                    return Err(stopped(advice, e));
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
    Err(no_server(servers.len(), last_error))
}

/// How the developer session came, for the line that says it's open.
#[derive(Clone, Copy)]
enum Signed {
    WithPassword,
    /// The token saved with the account, so nothing was asked.
    WithoutPassword,
}

/// What's left once Apple has opened a developer session: remembering the
/// server and the Apple Account, the team, and the sideloader that signs.
async fn signed_in(
    args: &Args,
    storage: &Path,
    apple_id: &str,
    server: &str,
    mut developer: DeveloperSession,
    teams: Vec<DeveloperTeam>,
    how: Signed,
) -> Result<Session> {
    remember_server(&args.home, server);
    // Before the team, so a sign-in that went through is kept even if no
    // team is picked.
    let new = remember_account(&args.home, apple_id);
    let (picked, kind, first) = pick_team(&mut developer, teams).await?;
    let team = team::summary(picked.name.as_deref(), &picked.team_id, kind);
    let mut sideloader = SideloaderBuilder::new(developer, apple_id.to_string())
        .team_selection(TeamSelection::First)
        .max_certs_behavior(MaxCertsBehavior::Prompt(Box::new(
            choose_certificates_to_revoke,
        )))
        .storage(Box::new(FsStorage::new(storage.to_path_buf())))
        .machine_name(MACHINE_NAME.to_string())
        .build();
    // `open` has just listed the teams, so the one picked is handed over
    // rather than listed again.
    sideloader.set_team(picked);
    ui::ok(&match how {
        Signed::WithPassword => format!("signed in — {team}"),
        Signed::WithoutPassword => format!("signed in without the password — {team}"),
    });
    if new {
        ui::note(saved::SAVED);
    }
    Ok(Session {
        sideloader,
        team,
        storage: storage.to_path_buf(),
        anisette_url: server.to_string(),
        kind,
        first,
    })
}

fn stopped(advice: &'static str, error: String) -> Fail {
    let mut f = Fail::new(advice, error);
    f.by_user = advice == CANCELLED;
    f
}

fn no_server(servers: usize, last_error: String) -> Fail {
    Fail::new(
        format!("Couldn't sign in through any of {servers} anisette servers. Try again later."),
        last_error,
    )
}

fn ask_email() -> Option<String> {
    ui::ask_text(
        "sign in to your Apple Account",
        "Email",
        "your Apple Account",
        "Apple Account email: ",
    )
}

fn ask_password() -> Result<String> {
    let password = ui::ask_secret(
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
    .unwrap_or_default();
    if password.is_empty() {
        return Err(Fail::by_user("No password given."));
    }
    Ok(password)
}

// ------------------------------------------------- remembered Apple Accounts

/// The Apple Account to sign in with when none was given: one of those saved,
/// picked from a menu that can also forget one, or typed. None when the user
/// stops.
fn which_account(home: &Path, storage: &Path) -> Option<String> {
    loop {
        let accounts = saved::list(home);
        if accounts.is_empty() {
            return ask_email();
        }
        let index = ui::choose(saved::QUESTION, &saved::options(&accounts))?;
        match saved::picked(index, accounts.len()) {
            saved::Pick::Account(i) => return accounts.get(i).cloned(),
            saved::Pick::Another => {
                if let Some(email) = ask_email() {
                    return Some(email);
                }
                // Back from the field: to the menu.
            }
            saved::Pick::Forget => forget_one(home, storage, &accounts),
        }
    }
}

/// The first time, the Apple Accounts with a sign-in saved here already
/// (ssboot 0.3.4 and older, or siboot before 0.20.0, whose store ssboot may start from, kept the token but no list) start the list.
fn start_list(home: &Path, storage: &Path) {
    if saved::exists(home) {
        return;
    }
    if let Err(e) = saved::start(home, &signed_in_before(storage)) {
        ui::detail(&format!("couldn't save the list of Apple Accounts: {e}"));
    }
}

/// Apple Accounts with a sign-in saved in isideload's store, the latest
/// first. Only a sign-in in its own account's folder counts, so the email it
/// names is that account's.
fn signed_in_before(storage: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(storage) else {
        return Vec::new();
    };
    let mut found: Vec<(u64, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let token = SavedToken::read(&entry.path().join(SESSION_FILE))?;
            (apple_session::account_dir(storage, &token.apple_id) == entry.path())
                .then_some((token.saved_at, token.apple_id))
        })
        .collect();
    found.sort_by_key(|(saved_at, _)| std::cmp::Reverse(*saved_at));
    found.into_iter().map(|(_, apple_id)| apple_id).collect()
}

/// Puts the Apple Account first on the list. True when it wasn't on it, so
/// the user hears it's kept.
fn remember_account(home: &Path, apple_id: &str) -> bool {
    match saved::remember(home, apple_id) {
        Ok(new) => new,
        Err(e) => {
            ui::detail(&format!(
                "couldn't save the Apple Account for next time: {e}"
            ));
            false
        }
    }
}

/// Forgets one of `accounts`, and the sign-in saved with it, once the user has
/// said which and confirmed. Its signing key stays, as SideInstaller keeps one
/// for an Apple ID it removes: signing in again then needs no new certificate.
fn forget_one(home: &Path, storage: &Path, accounts: &[String]) {
    let index = match accounts {
        [_] => Some(0),
        _ => ui::choose(saved::FORGET_WHICH, accounts),
    };
    let Some(email) = index.and_then(|i| accounts.get(i)) else {
        return;
    };
    if !ui::confirm(&format!("forget {email}?"), false) {
        return;
    }
    match saved::forget(home, email) {
        Ok(_) => {
            if apple_session::forget(storage, email) {
                ui::ok(&format!("forgot {email} and its saved sign-in"));
            } else {
                ui::ok(&format!("forgot {email}"));
            }
        }
        Err(e) => ui::warn(&format!("couldn't forget {email}: {e}")),
    }
}

// ------------------------------------------------------ the saved sign-in

/// apple_session.rs keeps an account's developer token in this file, in the
/// account's folder beside its signing key.
const SESSION_FILE: &str = "developer_session.json";

/// Signs in again this long before Apple's stated expiry, as apple_session.rs
/// does, rather than race it.
const EXPIRY_MARGIN_SECS: u64 = 10 * 60;

/// The developer token apple_session.rs saved after a sign-in, as it writes it
/// (its `SavedSession`). No `Debug`, so the token can't reach a log line.
#[derive(Deserialize)]
struct SavedToken {
    apple_id: String,
    adsid: String,
    token: String,
    /// Both as Apple sent them.
    duration: u64,
    expiry: u64,
    /// Unix seconds.
    saved_at: u64,
}

impl SavedToken {
    fn read(path: &Path) -> Option<SavedToken> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    /// When the token expires, as apple_session.rs reads it: Apple's stated
    /// expiry, else its duration from when it was saved. None when Apple
    /// gave neither: then it's tried anyway.
    fn expires_at(&self) -> Option<u64> {
        apple_session::stated_expiry(self.expiry)
            .or_else(|| (self.duration > 0).then(|| self.saved_at.saturating_add(self.duration)))
    }

    fn app_token(&self) -> AppToken {
        AppToken {
            token: self.token.clone(),
            duration: self.duration,
            expiry: self.expiry,
        }
    }

    /// For the details: how long Apple said it lasts from `now`.
    fn lifetime(&self, now: u64) -> String {
        match self.expires_at() {
            Some(at) if at > now => {
                let left = at - now;
                format!(
                    "Apple's sign-in lasts {}d {}h more",
                    left / 86_400,
                    left % 86_400 / 3_600
                )
            }
            Some(_) => "Apple's sign-in is past its expiry".into(),
            None => "Apple gave the sign-in no expiry".into(),
        }
    }
}

enum Saved {
    /// A token that hasn't expired: worth trying before the password.
    Usable(SavedToken),
    /// One that has: Apple needs the password again.
    RunOut,
    /// None, or one that can't be read or names another account.
    None,
}

fn saved_token(storage: &Path, apple_id: &str, now: u64) -> Saved {
    let path = apple_session::account_dir(storage, apple_id).join(SESSION_FILE);
    match SavedToken::read(&path) {
        Some(token) if token.apple_id != apple_id => Saved::None,
        Some(token)
            if token
                .expires_at()
                .is_some_and(|at| now.saturating_add(EXPIRY_MARGIN_SECS) >= at) =>
        {
            Saved::RunOut
        }
        Some(token) => Saved::Usable(token),
        None => Saved::None,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What the saved token came to.
enum ByToken {
    /// Apple opened a developer session with it, through this server.
    Opened(String, DeveloperSession, Vec<DeveloperTeam>),
    /// Apple turned it down: only the password signs in now.
    Refused,
}

/// Signs in with the saved token through each server in turn, as the
/// password would, never logging in. Stops as `sign_in` does when no server
/// gets it to Apple, or Apple is limiting sign-ins: the password wouldn't
/// help either, as `open` doesn't log in for those.
async fn with_token(token: &SavedToken, servers: &[String], storage: &Path) -> Result<ByToken> {
    let mut last_error = String::new();
    for server in servers {
        ui::status(&format!("signing in through {}", host_of(server)));
        match open_with_token(token, server, storage).await {
            Ok((developer, teams)) => {
                let lifetime = token.lifetime(unix_now());
                tracing::info!(
                    "Apple ID: signed in with the saved token, so no password ({lifetime})"
                );
                ui::detail(&format!("used the saved sign-in: {lifetime}"));
                return Ok(ByToken::Opened(server.clone(), developer, teams));
            }
            Err(Tried::Answered(e)) if refuses_token(&e) => {
                tracing::info!(
                    "Apple ID: Apple no longer takes the saved token ({}); asking for the password",
                    first_line(&e)
                );
                return Ok(ByToken::Refused);
            }
            Err(Tried::Answered(e) | Tried::NotReached(e)) => {
                if let Some(advice) = stops_sign_in(&e) {
                    return Err(stopped(advice, e));
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
    Err(no_server(servers.len(), last_error))
}

/// Why the token didn't open a session through a server.
enum Tried {
    /// The anisette server, the network or the client failed before Apple
    /// had it.
    NotReached(String),
    /// What Apple's developer portal answered.
    Answered(String),
}

/// The saved token through `server`, tried as `apple_session::open` tries it
/// before it logs in: on a GrandSlam client without the URL bag first, since
/// the developer portal doesn't need it, then on one with it, which can also
/// provision the anisette identity.
async fn open_with_token(
    token: &SavedToken,
    server: &str,
    storage: &Path,
) -> std::result::Result<(DeveloperSession, Vec<DeveloperTeam>), Tried> {
    let anisette = || {
        RemoteV3AnisetteProvider::new(
            server,
            Box::new(FsStorage::new(storage.to_path_buf())),
            "0".to_string(),
        )
        .map_err(|e| Tried::NotReached(format!("anisette provider: {}", report_text(&e))))
    };

    let without_url_bag = async {
        let provider = anisette()?;
        let client_info = provider
            .get_client_info()
            .await
            .map_err(|e| Tried::NotReached(format!("anisette client info: {}", report_text(&e))))?;
        let client = GrandSlam::without_url_bag(client_info, false)
            .map_err(|e| Tried::NotReached(format!("GrandSlam client: {}", report_text(&e))))?;
        let mut developer = DeveloperSession::new(
            token.app_token(),
            token.adsid.clone(),
            Arc::new(client),
            AnisetteDataGenerator::new(Arc::new(RwLock::new(provider))),
        );
        let teams = developer
            .list_teams()
            .await
            .map_err(|e| Tried::Answered(report_text(&e)))?;
        Ok::<_, Tried>((developer, teams))
    };
    match without_url_bag.await {
        Ok(opened) => return Ok(opened),
        Err(Tried::NotReached(e) | Tried::Answered(e)) => tracing::info!(
            "Apple ID: couldn't use the saved token straight away ({}); trying it with a full sign-in client",
            first_line(&e)
        ),
    }

    // Only fetches the anisette server's client info and the URL bag; no
    // sign-in.
    let account = AppleAccount::builder(&token.apple_id)
        .anisette_provider(anisette()?)
        .build()
        .await
        .map_err(|e| {
            Tried::NotReached(format!(
                "anisette client info or URL bag: {}",
                report_text(&e)
            ))
        })?;
    let mut developer = DeveloperSession::new(
        token.app_token(),
        token.adsid.clone(),
        account.grandslam_client.clone(),
        account.anisette_generator.clone(),
    );
    let teams = developer
        .list_teams()
        .await
        .map_err(|e| Tried::Answered(format!("developer session: {}", report_text(&e))))?;
    Ok((developer, teams))
}

/// Whether Apple's answer turned the token down, so only the password signs
/// in now. Not when the anisette server or the network failed on the way,
/// Apple was busy, or it refused the Apple Account itself (1102, 2160):
/// `open` doesn't log in again for those either (its `is_transient` and
/// `is_account_refusal`).
fn refuses_token(error: &str) -> bool {
    let e = error.to_lowercase();
    let transient = [
        "anisette",
        "error sending request",
        "timed out",
        "(429 ",
        "server error (5",
    ]
    .iter()
    .any(|s| e.contains(s));
    let account = e.contains("developer error 1102") || e.contains("developer error 2160");
    !transient && !account
}

/// The team that signs: the only one Apple lists, or the one picked when it
/// lists more (a paid membership and the free team every account has, or a
/// company's team), each marked free or paid. Apple's order is kept, so the
/// menu starts on the first: the team siboot took before 0.19.0, and the one
/// SideInstaller renews itself with. Also says whether that's the one picked.
async fn pick_team(
    developer: &mut DeveloperSession,
    mut teams: Vec<DeveloperTeam>,
) -> Result<(DeveloperTeam, Kind, bool)> {
    let none = "Signed in, but Apple didn't list a developer team.";
    if teams.is_empty() {
        // Asked once more, as isideload's `get_team` did before 0.19.0.
        teams = developer
            .list_teams()
            .await
            .map_err(|e| Fail::new(none, crate::error_text::report_text(&e)))?;
    }
    if teams.len() < 2 {
        let only = teams
            .pop()
            .ok_or_else(|| Fail::new(none, "No developer teams available"))?;
        return Ok((only, Kind::Unknown, true));
    }
    ui::status("reading your teams");
    let known = match list_kinds(developer).await {
        Ok(known) => known,
        Err(e) => {
            ui::detail(&format!(
                "couldn't tell free teams from paid ones: {}",
                first_line(&crate::error_text::report_text(&e))
            ));
            HashMap::new()
        }
    };
    let kinds: Vec<Kind> = teams
        .iter()
        .map(|t| known.get(&t.team_id).copied().unwrap_or(Kind::Unknown))
        .collect();
    let labels: Vec<String> = teams
        .iter()
        .zip(&kinds)
        .map(|(t, kind)| team::label(t.name.as_deref(), &t.team_id, *kind))
        .collect();
    ui::status(&team::found(teams.len()));
    ui::note(team::NOTE);
    match ui::choose(team::QUESTION, &labels) {
        Some(i) if i < teams.len() => Ok((teams.swap_remove(i), kinds[i], i == 0)),
        _ => Err(Fail::by_user("No developer team picked.")),
    }
}

/// Each team's kind, from Apple's listTeams asked again: isideload's
/// `DeveloperTeam` leaves out the memberships, and they're what tells a free
/// team from a paid one. Only asked when there's a choice to make.
async fn list_kinds(
    developer: &mut DeveloperSession,
) -> std::result::Result<HashMap<String, Kind>, Report> {
    let listed: Vec<team::Listed> = developer
        .send_dev_request(
            &dev_url("listTeams", DeveloperDeviceType::Any),
            None,
            "teams",
        )
        .await?;
    Ok(team::kinds(&listed))
}

/// `pick` alone when given; otherwise the last server that worked, then the
/// rest of `SERVERS`: when the last one was a macOS server other than lillq's
/// (`POOL`), the others that take its identity come before lillq, as in
/// SideInstaller's `AnisetteServer.candidates`.
fn candidates(pick: Option<&str>, last: Option<&str>) -> Vec<String> {
    if let Some(pick) = pick {
        return vec![pick.trim().trim_end_matches('/').to_string()];
    }
    let trim = |s: &str| s.trim().trim_end_matches('/').to_string();
    let last = last.map(trim).filter(|s| !s.is_empty());
    let shares_identity = |s: &str| s != POOL && s != WINDOWS_SERVER;
    let mut order: Vec<String> = last.iter().cloned().collect();
    if last.as_deref().is_some_and(|l| SERVERS.contains(&l) && shares_identity(l)) {
        order.extend(SERVERS.iter().filter(|s| shares_identity(**s)).map(|s| s.to_string()));
    }
    order.extend(SERVERS.iter().map(|s| s.to_string()));
    let mut out: Vec<String> = Vec::new();
    for s in order {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// The one server in `SERVERS` on Apple's Windows library.
const WINDOWS_SERVER: &str = "https://win.ani.sidestore.io";

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

/// isideload keeps an account's signing key and saved sign-in in
/// `<sha256(email)>/`. One made under the email as typed, before siboot
/// lowercased it, is moved to the lowercase name, so the account keeps its key
/// and certificate instead of making another.
fn adopt_folder_typed_as(storage: &Path, typed: &str, apple_id: &str) {
    if typed == apple_id {
        return;
    }
    let old = apple_session::account_dir(storage, typed);
    let new = apple_session::account_dir(storage, apple_id);
    if old.is_dir() && !new.exists() {
        let _ = std::fs::rename(old, new);
    }
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
    if ui::interactive() {
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
        if !ui::interactive() {
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
            candidates(None, None)[..2],
            ["https://anisette.lillq.me", "https://new.ani.sidestore.io"]
        );
        // lillq's identities work nowhere else: after a macOS server, the
        // others that take its identity (and its saved token) come first.
        let after_mac = candidates(None, Some("https://new.ani.sidestore.io"));
        let lillq = after_mac.iter().position(|s| s == POOL).unwrap();
        assert_eq!(lillq, SERVERS.len() - 2, "{after_mac:?}");
        assert_eq!(after_mac.last().unwrap(), WINDOWS_SERVER);
        assert_eq!(after_mac.len(), SERVERS.len());
        // After lillq or the Windows server, lillq's next in line anyway.
        assert_eq!(candidates(None, Some(POOL)), SERVERS);
        assert_eq!(candidates(None, Some(WINDOWS_SERVER))[1], POOL);
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

    #[test]
    fn a_folder_made_under_a_capitalized_email_is_kept() {
        let storage = std::env::temp_dir().join(format!("ssboot-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&storage);
        let typed = "Someone@Example.com";
        let apple_id = typed.to_lowercase();
        let old = apple_session::account_dir(&storage, typed);
        let new = apple_session::account_dir(&storage, &apple_id);

        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("key"), "old key").unwrap();
        adopt_folder_typed_as(&storage, typed, &apple_id);
        assert!(!old.exists());
        assert_eq!(std::fs::read_to_string(new.join("key")).unwrap(), "old key");

        // A lowercase folder already there is never replaced.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("key"), "other key").unwrap();
        adopt_folder_typed_as(&storage, typed, &apple_id);
        assert_eq!(std::fs::read_to_string(new.join("key")).unwrap(), "old key");
        assert!(old.exists());

        std::fs::remove_dir_all(&storage).unwrap();
    }

    const NOW: u64 = 1_789_300_000;

    fn store(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ssboot-token-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A token as apple_session.rs's `SavedSession` serializes, in the
    /// account's folder.
    fn save_token(storage: &Path, apple_id: &str, expiry: u64, duration: u64, saved_at: u64) {
        let dir = apple_session::account_dir(storage, apple_id);
        std::fs::create_dir_all(&dir).unwrap();
        let json = format!(
            r#"{{"apple_id":"{apple_id}","adsid":"001234-05-abcdef","token":"AAAABLwIAAAAAGW","duration":{duration},"expiry":{expiry},"saved_at":{saved_at}}}"#
        );
        std::fs::write(dir.join(SESSION_FILE), json).unwrap();
    }

    #[test]
    fn a_saved_token_is_tried_until_it_runs_out() {
        let storage = store("expiry");
        let id = "someone@example.com";
        assert!(matches!(saved_token(&storage, id, NOW), Saved::None));

        // Apple's expiry in milliseconds, a day off.
        save_token(&storage, id, (NOW + 86_400) * 1000, 0, NOW - 100);
        let Saved::Usable(token) = saved_token(&storage, id, NOW) else {
            panic!("a day left is usable");
        };
        assert_eq!(token.adsid, "001234-05-abcdef");
        assert_eq!(token.app_token().token, "AAAABLwIAAAAAGW");
        assert_eq!(token.lifetime(NOW), "Apple's sign-in lasts 1d 0h more");

        // Within ten minutes of the expiry, or past it: the password.
        save_token(&storage, id, NOW + 300, 0, NOW - 100);
        assert!(matches!(saved_token(&storage, id, NOW), Saved::RunOut));
        save_token(&storage, id, 0, 3_600, NOW - 7_200);
        assert!(matches!(saved_token(&storage, id, NOW), Saved::RunOut));

        // No expiry and no duration: tried anyway, as apple_session.rs does.
        save_token(&storage, id, 0, 0, NOW - 100);
        assert!(matches!(saved_token(&storage, id, NOW), Saved::Usable(_)));

        // One naming another account, or one that can't be read: none.
        save_token(&storage, id, 0, 0, NOW);
        let dir = apple_session::account_dir(&storage, id);
        let other = std::fs::read_to_string(dir.join(SESSION_FILE))
            .unwrap()
            .replace(id, "someone.else@example.com");
        std::fs::write(dir.join(SESSION_FILE), other).unwrap();
        assert!(matches!(saved_token(&storage, id, NOW), Saved::None));
        std::fs::write(dir.join(SESSION_FILE), "{not json").unwrap();
        assert!(matches!(saved_token(&storage, id, NOW), Saved::None));

        std::fs::remove_dir_all(&storage).unwrap();
    }

    /// siboot reads the token where apple_session.rs keeps it: its own
    /// `forget` takes away the file siboot reads.
    #[test]
    fn the_token_is_read_where_apple_session_keeps_it() {
        let storage = store("path");
        let id = "someone@example.com";
        save_token(&storage, id, 0, 0, NOW);
        assert!(matches!(saved_token(&storage, id, NOW), Saved::Usable(_)));
        assert!(apple_session::forget(&storage, id));
        assert!(matches!(saved_token(&storage, id, NOW), Saved::None));
        std::fs::remove_dir_all(&storage).unwrap();
    }

    #[test]
    fn accounts_that_signed_in_before_start_the_list_latest_first() {
        let storage = store("before");
        let home = storage.join("home");
        save_token(&storage, "old@example.com", 0, 0, NOW - 1_000);
        save_token(&storage, "new@example.com", 0, 0, NOW);
        // A token in a folder that isn't its account's is left out.
        save_token(&storage, "moved@example.com", 0, 0, NOW + 1);
        std::fs::rename(
            apple_session::account_dir(&storage, "moved@example.com"),
            storage.join("elsewhere"),
        )
        .unwrap();
        // So is a folder with no token, like the anisette identity's.
        std::fs::create_dir_all(storage.join("anisette")).unwrap();

        assert_eq!(
            signed_in_before(&storage),
            ["new@example.com", "old@example.com"]
        );
        start_list(&home, &storage);
        assert_eq!(saved::list(&home), ["new@example.com", "old@example.com"]);

        // Only the first time: one forgotten since doesn't come back.
        saved::forget(&home, "old@example.com").unwrap();
        start_list(&home, &storage);
        assert_eq!(saved::list(&home), ["new@example.com"]);

        std::fs::remove_dir_all(&storage).unwrap();
    }

    #[test]
    fn a_new_account_is_said_to_be_kept_once() {
        let home = store("remember");
        assert!(remember_account(&home, "someone@example.com"));
        assert!(!remember_account(&home, "someone@example.com"));
        assert_eq!(saved::list(&home), ["someone@example.com"]);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn only_apples_refusal_of_the_token_asks_for_the_password() {
        for refused in [
            " ● Developer error 1100: Your session has expired. Please log in.",
            " ● Developer request failed\n ● HTTP status client error (401 Unauthorized) for url (https://developerservices2.apple.com/)",
            "developer session:  ● Failed to extract developer request result",
        ] {
            assert!(refuses_token(refused), "{refused}");
        }
        for not_refused in [
            " ● Failed to get anisette headers\n ● HTTP status server error (522 <unknown status code>)",
            " ● error sending request for url (https://developerservices2.apple.com/services/QH65B2/listTeams.action)",
            " ● HTTP status client error (429 Too Many Requests) for url (https://developerservices2.apple.com/)",
            " ● Developer request failed\n ● HTTP status server error (503 Service Unavailable)",
            " ● Failed to list developer teams\n ● Developer error 2160: Please verify the email address associated with your Apple Account.",
            " ● Developer error 1102: (Apple's age refusal)",
        ] {
            assert!(!refuses_token(not_refused), "{not_refused}");
        }
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
