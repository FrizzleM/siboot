//! Step 2: a pairing this iPhone accepts, made here in Open Minis.
//!
//! From iOS 27 an iPhone pair-sets-up only with a host it dials itself: one
//! advertising `_remotepairing-pairable-host._tcp`, which Settings › Privacy &
//! Security › Developer Mode lists as "Pair with …". Tapping that has the
//! iPhone dial the advertised port and drive pair-setup with the roles
//! flipped: the iPhone is the rppairing "host", and siboot answers as the
//! accessory and shows the code. Dialling the iPhone instead can only
//! pair-*verify*: it answers `allowsPairSetup: false`.
//!
//! The shell can't advertise (iOS refuses its multicast), but Open Minis 1.16
//! does it for the shell: `minis-bonjour advertise` (bridge.rs) publishes from
//! the app and forwards each connection to siboot's port on the loopback.
//! siboot answers with idevice's `PairableHost`, the responder SideInstaller
//! uses, and plays silence through Minis's player so iOS keeps Minis running
//! while the user is in Settings (keepalive.rs).
//!
//! The pairing comes from, in order:
//! 1. the file saved on an earlier run, while the iPhone accepts it;
//! 2. the file siboot-pair offers from rootshell (../rootshell), for the
//!    transfer code it shows: the route from before Minis could pair, still
//!    taken when rootshell is offering;
//! 3. a new pairing, here in Minis.
//!
//! Either way the iPhone has to accept the file before anything is signed.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use idevice::remote_pairing::{
    PairableHost, PairableHostInfo, PeerDevice, RpPairingFile, RpPairingSocket,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use crate::bridge::{Advertisement, Bridge, BridgeError, PAIRABLE_HOST_TYPE};
use crate::fail::{chain, Fail, Result};
use crate::keepalive::{self, KeepAlive};
use crate::{tunnel, ui, Args};

/// Where siboot-pair offers the file (rootshell/src/main.rs, `HANDOFF_PORT`).
const HANDOFF_PORT: u16 = 61207;
/// Codes typed before siboot gives up. siboot-pair ends its offer after five.
const CODE_TRIES: u32 = 3;
/// Far larger than a pairing file, which is about 1 KB.
const MAX_FILE: usize = 64 * 1024;
/// How long siboot-pair may take to answer a code. It answers at once while
/// rootshell runs, so silence means iOS has paused it.
const ANSWER_WAIT: Duration = Duration::from_secs(20);

/// "Pair with siboot" in Settings, as siboot-pair calls itself too.
pub const HOST_NAME: &str = "siboot";
/// What the iPhone reads off the host. iOS treats a pairable host as a
/// computer, so the model stays a Mac's, as rust-core and StikPair send.
const HOST_MODEL: &str = "Mac17,7";

/// How long each stage of a pairing may take.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// For the user to find the entry in Settings and tap it.
    pub device: Duration,
    /// From the iPhone connecting until it asks for a code. In SideInstaller's
    /// logs both arrive within 40 ms of each other.
    pub code_request: Duration,
    /// From the code being shown until pairing has finished.
    pub code_entry: Duration,
}

pub const LIMITS: Limits = Limits {
    device: Duration::from_secs(10 * 60),
    code_request: Duration::from_secs(30),
    code_entry: Duration::from_secs(5 * 60),
};

/// Connections tried before giving up: a stray or aborted one, a mistyped
/// code, and the one that pairs.
const MAX_ATTEMPTS: u32 = 3;

/// How long a paired run waits for the user to come back to Minis before
/// carrying on regardless.
const RETURN_WAIT: Duration = Duration::from_secs(10 * 60);

/// What to do where this Minis can't pair: update it, or pair in rootshell.
pub const UPDATE_OR_ROOTSHELL: &str = "Update Open Minis from the App Store (1.16 and later pair \
     this iPhone themselves) and run siboot again. Or pair in rootshell (App Store): turn on its \
     Settings › Location Diary Mode › Session Only, then run in its local shell, one line at a \
     time: `curl -fsSLO https://raw.githubusercontent.com/FrizzleM/siboot/main/rootshell/bin/siboot-pair.wasm`, \
     `ifconfig en0 > ifconfig.txt`, `./siboot-pair.wasm`. Once it has paired, leave it running \
     and run siboot here again: it asks for the transfer code rootshell shows.";

/// Where the pairing that works came from.
pub enum Source {
    /// The file saved on an earlier run.
    Saved,
    /// siboot-pair's offer in rootshell.
    Rootshell,
    /// A new pairing, made here.
    Minis,
}

pub async fn ensure(args: &Args, path: &Path) -> Result<Source> {
    let mut saved_refused = None;
    if path.exists() {
        ui::activity("trying the pairing file saved on an earlier run");
        match tunnel::verify(args.device, path).await {
            Ok(()) => {
                ui::ok("this iPhone accepts the pairing file saved on an earlier run");
                return Ok(Source::Saved);
            }
            Err(f) => {
                ui::note("This iPhone doesn't accept the saved pairing file, so siboot pairs again.");
                ui::detail(&f.raw);
                saved_refused = Some(f);
            }
        }
    }

    let bridge = Bridge::from_env();
    ui::activity("checking that Open Minis can pair this iPhone");
    let here = can_pair_here(&bridge).await;

    let offer = SocketAddr::from(([127, 0, 0, 1], HANDOFF_PORT));
    let hint = if here.is_ok() {
        "type the transfer code it shows, or Cancel to pair here in Minis instead"
    } else {
        "type the transfer code it shows"
    };
    let ask = || {
        ui::ask_code(
            "rootshell is offering its file",
            "Code",
            hint,
            "Transfer code shown in rootshell: ",
        )
    };
    match take(offer, path, ask, ANSWER_WAIT).await? {
        Taken::File => {
            ui::activity("checking it with this iPhone");
            tunnel::verify(args.device, path).await?;
            ui::ok("this iPhone accepts the pairing file from rootshell");
            return Ok(Source::Rootshell);
        }
        Taken::Cancelled if here.is_err() => {
            return Err(Fail::by_user("Stopped at the transfer code."))
        }
        Taken::Cancelled | Taken::NotOffered => {}
    }

    let notify = match here {
        Ok(notify) => notify,
        Err(why) => {
            let raw = match saved_refused {
                Some(f) => format!("the saved pairing file was refused: {}", f.raw),
                None => format!("nothing offering on {offer}, and no {}", path.display()),
            };
            return Err(Fail::new(format!("{why} {UPDATE_OR_ROOTSHELL}"), raw));
        }
    };
    ui::ok("Open Minis can pair this iPhone");
    pair_here(args, path, &bridge, notify).await?;
    ui::activity("checking the new pairing with this iPhone");
    tunnel::verify(args.device, path).await.map_err(|f| {
        Fail::new(
            format!("Paired, but this iPhone doesn't accept the new pairing: {}", f.advice),
            f.raw,
        )
    })?;
    ui::ok("this iPhone accepts the new pairing");
    Ok(Source::Minis)
}

/// Whether this Open Minis can advertise a pairing host: Ok with whether it
/// can post the code as a notification too, or why it can't pair.
pub async fn can_pair_here(bridge: &Bridge) -> std::result::Result<bool, String> {
    let types = match bridge.bonjour_types().await {
        Ok(types) => types,
        Err(BridgeError::Missing(_)) => {
            return Err("This Open Minis can't pair this iPhone itself: that came with Minis 1.16."
                .to_string())
        }
        Err(e) => return Err(format!("Open Minis's minis-bonjour didn't answer ({e}).")),
    };
    if !types.declares(PAIRABLE_HOST_TYPE) {
        return Err("This Open Minis can't advertise a pairing host.".to_string());
    }
    ui::detail(&format!(
        "minis-bonjour can advertise {PAIRABLE_HOST_TYPE} ({} types declared, {})",
        types.types.len(),
        if types.restricted { "only those" } else { "any type allowed" }
    ));
    let notify = match bridge.notification_status().await {
        Ok(status) => {
            ui::detail(&format!("Minis's notifications: {status}"));
            status != "denied"
        }
        Err(e) => {
            ui::detail(&format!("couldn't read Minis's notification settings: {e}"));
            false
        }
    };
    Ok(notify)
}

// ---------------------------------------------------------------- pairing here

/// Pair this iPhone with siboot, here in Minis, into `path`.
async fn pair_here(args: &Args, path: &Path, bridge: &Bridge, notify: bool) -> Result<()> {
    let identity = Identity::load(path, &args.home).await;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|e| Fail::new("Couldn't open a port to listen on.", e.to_string()))?;
    let setup = HostSetup {
        bridge: bridge.clone(),
        advertise: true,
        name: HOST_NAME.to_string(),
        keepalive_dir: Some(args.home.clone()),
        notify,
        limits: LIMITS,
        pin_hook: None,
        player_check: PLAYER_CHECK,
    };
    let paired = run_host(listener, setup, identity, path, &args.home).await?;
    ui::ok(&format!("paired with {} ({})", paired.name, paired.model));
    ui::detail(&format!("its RemotePairing UDID is {}", paired.udid));
    Ok(())
}

/// The host identity, kept between runs.
///
/// A new key pair each run would invalidate every copy of the pairing file
/// already handed to other apps (SideInstaller included), and a new altIRK
/// would change the advertised `authTag`, so an iPhone that paired before
/// wouldn't recognise the host. rust-core keeps both for the same reasons.
pub struct Identity {
    pub file: RpPairingFile,
    pub alt_irk: Option<[u8; 16]>,
}

impl Identity {
    pub async fn load(path: &Path, home: &Path) -> Self {
        let file = match RpPairingFile::read_from_file(path).await {
            Ok(mut existing) => {
                // That was the iPhone's altIRK; the next pairing brings its own.
                existing.alt_irk = None;
                ui::detail("reusing the host key pair from the saved pairing file");
                existing
            }
            Err(_) => {
                let mut file = RpPairingFile::generate(HOST_NAME);
                // `generate` derives the identifier from the name, the same for
                // every siboot; a random one keeps this host's records apart.
                file.identifier = uuid::Uuid::new_v4().to_string().to_uppercase();
                file
            }
        };
        let alt_irk = std::fs::read_to_string(alt_irk_path(home))
            .ok()
            .and_then(|hex| parse_alt_irk(hex.trim()));
        Self { file, alt_irk }
    }
}

fn alt_irk_path(home: &Path) -> PathBuf {
    home.join("host_alt_irk")
}

/// How to run one host.
pub struct HostSetup {
    pub bridge: Bridge,
    /// Advertise through `minis-bonjour`. The tests dial the port directly.
    pub advertise: bool,
    /// "Pair with <name>".
    pub name: String,
    /// Play silence from here while waiting.
    pub keepalive_dir: Option<PathBuf>,
    /// Post the code as a Minis notification.
    pub notify: bool,
    pub limits: Limits,
    /// Tests: receives the code as it's shown.
    pub pin_hook: Option<mpsc::UnboundedSender<String>>,
    /// How often to check the silence still plays while waiting for Settings.
    pub player_check: Duration,
}

/// The player's sheet covers the page, so it may well be swiped away before
/// the user leaves for Settings.
const PLAYER_CHECK: Duration = Duration::from_secs(3);

pub struct Paired {
    pub name: String,
    pub model: String,
    pub udid: String,
}

/// Get the user ready, advertise `listener`, wait for the iPhone, pair, and
/// write the pairing file to `out_path`. The advertisement and the silence
/// end with this call, however it ends.
pub async fn run_host(
    listener: TcpListener,
    setup: HostSetup,
    identity: Identity,
    out_path: &Path,
    home: &Path,
) -> Result<Paired> {
    let Identity {
        file: mut pairing_file,
        alt_irk,
    } = identity;
    let port = listener
        .local_addr()
        .map_err(|e| Fail::new("Couldn't open a port to listen on.", e.to_string()))?
        .port();

    let mut host_info = PairableHostInfo::generate(setup.name.clone(), HOST_MODEL);
    if let Some(alt_irk) = alt_irk {
        host_info.alt_irk = alt_irk;
    }
    let service_id = pairing_file.identifier.clone();
    let txt = host_info.mdns_txt_records(&service_id);

    // Asked first, so nothing starts waiting on the user before they're
    // ready, and asked again after a refusal they can fix.
    let steps = ready_steps(&setup);
    let steps: Vec<&str> = steps.iter().map(String::as_str).collect();
    let notes: &[&str] = if setup.keepalive_dir.is_some() { &[PLAYER_NOTE] } else { &[] };
    if setup.advertise {
        withdraw_leftovers(&setup.bridge).await;
    }
    let advert = loop {
        ui::status("start pairing when you're ready");
        if !ui::get_ready(READY_TITLE, &steps, notes, "Start pairing") {
            return Err(Fail::by_user("Stopped before pairing."));
        }
        if !setup.advertise {
            break None;
        }
        match publish(&setup, &service_id, port, &txt).await {
            Ok(advert) => break Some(advert),
            Err(e) => match fixable(&e) {
                Some(advice) => {
                    ui::warn(advice);
                    ui::detail(&e.to_string());
                }
                None => return Err(advertise_failed(e)),
            },
        }
    };

    if setup.notify && setup.advertise {
        // Posted while Minis is still in front, so iOS can ask for permission
        // first, if it hasn't yet: behind Settings it couldn't.
        notify(
            &setup.bridge,
            &format!("Pair with {} is ready", setup.name),
            "Settings › Privacy & Security › Developer Mode, at the bottom.",
        );
    }
    let mut keepalive = match &setup.keepalive_dir {
        Some(dir) => match KeepAlive::start(&setup.bridge, dir, keepalive::PAIRING_SECONDS).await {
            Ok(k) => {
                ui::detail("playing silence through apple-player, so iOS keeps Minis running");
                Some(k)
            }
            Err(e) => {
                ui::warn(
                    "Couldn't start the silence that keeps Minis running behind Settings, so iOS \
                     may pause it there before Pair with siboot shows.",
                );
                ui::detail(&e);
                None
            }
        },
        None => None,
    };

    let waiting = format!(
        "waiting for Pair with {} in Settings (up to {})",
        setup.name,
        spoken(setup.limits.device)
    );
    ui::status(&waiting);
    let deadline = tokio::time::Instant::now() + setup.limits.device;
    let mut attempt = 0;
    let peer: PeerDevice = loop {
        attempt += 1;
        let accepted = loop {
            tokio::select! {
                accepted = tokio::time::timeout_at(deadline, listener.accept()) => break accepted,
                () = tokio::time::sleep(setup.player_check), if keepalive.is_some() => {
                    if let Some(k) = keepalive.as_mut() {
                        keep_playing(k, &setup).await;
                    }
                }
            }
        };
        let stream = match accepted {
            Ok(Ok((stream, peer))) => {
                ui::status("Settings connected: waiting for it to ask for a code");
                ui::detail(&format!("a connection from {peer}"));
                stream
            }
            Ok(Err(e)) => {
                return Err(Fail::new(
                    "Couldn't accept the iPhone's connection.",
                    e.to_string(),
                ))
            }
            Err(_) => {
                return Err(Fail::new(
                    format!(
                        "Nobody tapped Pair with {} within {}. If it never showed in Settings, \
                         keep Wi-Fi on, check Open Minis has Local Network access (Settings › \
                         Privacy & Security › Local Network), leave the player open, and run \
                         siboot again.",
                        setup.name,
                        spoken(setup.limits.device)
                    ),
                    "no connection to the pairing port",
                ))
            }
        };

        let socket = RpPairingSocket::new_device(stream);
        let mut host = PairableHost::new(socket, host_info.clone());
        let (code_shown_tx, code_shown) = oneshot::channel();
        let (code_seen_tx, mut code_seen) = oneshot::channel::<()>();
        let bridge = setup.bridge.clone();
        let posts = setup.notify;
        let hook = setup.pin_hook.clone();
        let ended = {
            let conversation = host.accept(&mut pairing_file, move |pin: String| async move {
                ui::status("waiting for the code to be typed into Settings");
                ui::show_code(
                    &pin,
                    if posts {
                        "Type it into Settings. It came as a notification too."
                    } else {
                        "Type it into Settings."
                    },
                );
                if posts {
                    notify(
                        &bridge,
                        "siboot pairing code",
                        &format!("{} — type it into Settings", ui::spaced(&pin)),
                    );
                }
                if let Some(hook) = hook {
                    let _ = hook.send(pin);
                }
                let _ = code_seen_tx.send(());
                let _ = code_shown_tx.send(());
            });
            within_limits(conversation, code_shown, setup.limits).await
        };
        ui::code_gone();
        // A stray or aborted connection, a mistyped code, a cancel in
        // Settings: the user can tap the entry again, and the advertisement is
        // still up, so the run waits for that rather than ending.
        let code_was_seen = code_seen.try_recv().is_ok();
        let (failure, note) = match ended {
            Ok(Ok(peer)) => break peer,
            // The code wasn't typed in time: waiting longer won't help.
            Err(timeout) if code_was_seen => return Err(timeout),
            Err(quiet) => (quiet, "That connection went quiet before asking for a code."),
            Ok(Err(e)) if !code_was_seen => (
                pairing_failed(&e),
                "That connection closed before asking for a code.",
            ),
            Ok(Err(e)) if is_wrong_code(&e) => (
                pairing_failed(&e),
                "That code didn't match. Tap the entry in Settings again for a new one.",
            ),
            Ok(Err(e)) => (
                pairing_failed(&e),
                "Pairing stopped before it finished. Tap the entry in Settings again.",
            ),
        };
        if attempt >= MAX_ATTEMPTS {
            return Err(failure);
        }
        ui::warn(note);
        ui::detail(&failure.raw);
        ui::status(&waiting);
    };
    drop(listener);

    save(out_path, &pairing_file.to_bytes())?;
    ui::detail(&format!("pairing file written to {}", out_path.display()));
    if let Err(e) = std::fs::write(alt_irk_path(home), hex::encode(host_info.alt_irk)) {
        ui::detail(&format!("couldn't keep the host altIRK: {e}"));
    }

    // Withdrawn now rather than at the end of the run: the entry must not
    // outlive the listener behind it.
    if let Some(advert) = advert {
        advert.stop().await;
    }
    if let Some(k) = keepalive {
        // Pairing ends the moment the code is typed, with Minis still behind
        // Settings. Stopping the silence then lets iOS pause Minis mid-step,
        // and closes the player's sheet while Minis is in the background,
        // after which its terminal can stop redrawing. So the silence plays on
        // until the user is back in Minis.
        wait_for_return(&setup).await;
        k.stop().await;
    }

    Ok(Paired {
        name: peer.name,
        model: peer.model,
        udid: peer.remotepairing_udid,
    })
}

const READY_TITLE: &str = "Pair this iPhone with siboot";
const PLAYER_NOTE: &str = "When you start, a player opens over Minis. It plays silence, which \
     keeps Minis running while you're in Settings: leave it open. siboot closes it once you're \
     back.";

/// What the user does in Settings, in order.
fn ready_steps(setup: &HostSetup) -> Vec<String> {
    vec![
        "Open Settings › Privacy & Security › Developer Mode, and scroll to the bottom.".into(),
        format!(
            "Tap Pair with {}. It can take a few seconds to show.",
            setup.name
        ),
        if setup.notify {
            "Enter your iPhone passcode, then the 6-digit code siboot shows. It comes as a \
             notification too."
                .into()
        } else {
            "Enter your iPhone passcode, then the 6-digit code siboot shows. Minis's \
             notifications are off, so come back to Minis to read it."
                .into()
        },
        "Come back to Minis once Settings says it's paired.".into(),
    ]
}

/// Publish the pairing host through Minis, under the identifier as its name:
/// the iPhone checks the one against the other.
async fn publish(
    setup: &HostSetup,
    service_id: &str,
    port: u16,
    txt: &[(String, String)],
) -> std::result::Result<Advertisement, BridgeError> {
    let advert = setup
        .bridge
        .advertise(PAIRABLE_HOST_TYPE, service_id, port, txt)
        .await?;
    let a = advert.advert().clone();
    if a.name != service_id {
        advert.stop().await;
        return Err(BridgeError::Refused {
            code: "renamed".into(),
            message: format!("asked for {service_id}, got {}", a.name),
        });
    }
    ui::ok(&format!("Pair with {} is up in Settings", setup.name));
    ui::detail(&format!(
        "{PAIRABLE_HOST_TYPE} {} on Minis's port {} → port {port} here",
        a.name,
        a.listener_port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".into())
    ));
    if let Some(w) = a.warning {
        ui::detail(&format!("minis-bonjour: {w}"));
    }
    Ok(advert)
}

/// The silence must play on while siboot waits for Settings, or iOS pauses
/// Minis there and Pair with siboot goes. Started again while Minis is in
/// front, where its player can open.
async fn keep_playing(keepalive: &mut KeepAlive, setup: &HostSetup) {
    let in_front = setup
        .bridge
        .bonjour_list()
        .await
        .map(|l| !l.app_in_background)
        .unwrap_or(true);
    if !in_front {
        return;
    }
    match keepalive.revive().await {
        Ok(false) => {}
        Ok(true) => ui::warn(
            "The player was closed. It keeps Minis running while you're in Settings, so siboot \
             opened it again: leave it open.",
        ),
        Err(e) => ui::detail(&format!("couldn't start the silence again: {e}")),
    }
}

/// A pairing host left by an earlier run that was killed would put a second
/// "Pair with …" in Settings, one that leads nowhere.
async fn withdraw_leftovers(bridge: &Bridge) {
    let listing = match bridge.bonjour_list().await {
        Ok(listing) => listing,
        Err(e) => {
            ui::detail(&format!("minis-bonjour list failed: {e}"));
            return;
        }
    };
    for advert in listing.advertisements.iter().filter(|a| a.is_pairable_host()) {
        ui::detail(&format!("withdrawing a pairing host left from before ({})", advert.name));
        if let Err(e) = bridge.stop_advert(&advert.id).await {
            ui::detail(&format!("couldn't withdraw {}: {e}", advert.id));
        }
    }
}

/// What the user can fix in Settings, then start again.
fn fixable(e: &BridgeError) -> Option<&'static str> {
    match e.code() {
        Some("authorization_denied") => Some(
            "Open Minis needs Local Network access to show Pair with siboot. Allow it when iOS \
             asks, or turn it on in Settings › Privacy & Security › Local Network, then start \
             again.",
        ),
        Some("disabled") => Some(
            "Local Discovery is off in Open Minis. Turn it on in Minis's Settings › Network, \
             then start again.",
        ),
        _ => None,
    }
}

fn advertise_failed(e: BridgeError) -> Fail {
    let advice = match e.code() {
        Some("not_declared") => {
            format!("This Open Minis can't advertise a pairing host. {UPDATE_OR_ROOTSHELL}")
        }
        Some("renamed") => "Minis put Pair with siboot up under another name, which Settings \
                            won't pair with. Run siboot again."
            .to_string(),
        _ => "Open Minis couldn't put Pair with siboot up in Settings. Run siboot again.".to_string(),
    };
    Fail::new(advice, e.to_string())
}

/// A notification from Minis, a second from now so it shows over Settings.
/// Sent off on its own: the pairing doesn't wait on it.
fn notify(bridge: &Bridge, title: &str, body: &str) {
    let bridge = bridge.clone();
    let (title, body) = (title.to_string(), body.to_string());
    tokio::spawn(async move {
        if let Err(e) = bridge.notify(&title, &body).await {
            tracing::warn!("couldn't post a notification: {e}");
        }
    });
}

/// Until Minis is in front again, as `minis-bonjour list` reports it, asking
/// the user back with a notification. Returns at once if Minis is already in
/// front, or if the bridge can't say.
async fn wait_for_return(setup: &HostSetup) {
    let in_background = |setup: &HostSetup| {
        let bridge = setup.bridge.clone();
        async move {
            bridge
                .bonjour_list()
                .await
                .map(|l| l.app_in_background)
                .unwrap_or(false)
        }
    };
    if !in_background(setup).await {
        return;
    }
    ui::status("paired: go back to Minis to carry on");
    if setup.notify {
        notify(
            &setup.bridge,
            "siboot paired this iPhone",
            "Go back to Minis: siboot carries on there.",
        );
    }
    let deadline = tokio::time::Instant::now() + RETURN_WAIT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if !in_background(setup).await {
            // Let Minis finish coming forward before its sheet is closed.
            tokio::time::sleep(Duration::from_millis(500)).await;
            return;
        }
    }
}

fn pairing_failed(e: &idevice::IdeviceError) -> Fail {
    let advice = if is_wrong_code(e) {
        "The code didn't match. Run siboot again and type the new code exactly."
    } else {
        "Pairing didn't finish. Run siboot again, and tap the entry again in Settings."
    };
    Fail::new(advice, chain(e))
}

/// idevice's `SrpAuthFailed`: the client's proof didn't match the code.
fn is_wrong_code(e: &idevice::IdeviceError) -> bool {
    chain(e).to_ascii_lowercase().contains("srp")
}

/// idevice's conversation waits on the iPhone without a limit:
/// `code_request` until the code is shown, then `code_entry` until it ends.
async fn within_limits<T>(
    conversation: impl Future<Output = T>,
    code_shown: oneshot::Receiver<()>,
    limits: Limits,
) -> Result<T> {
    tokio::pin!(conversation);
    tokio::select! {
        biased;
        done = &mut conversation => return Ok(done),
        // An error means the PIN callback was dropped uncalled, which only
        // happens as the conversation ends, so it disables this branch.
        Ok(()) = code_shown => {}
        () = tokio::time::sleep(limits.code_request) => {
            return Err(Fail::new(
                "The iPhone connected but stopped before asking for a code. Tap the entry in \
                 Settings again, or run siboot again.",
                format!("no code request within {}", spoken(limits.code_request)),
            ));
        }
    }
    tokio::time::timeout(limits.code_entry, conversation)
        .await
        .map_err(|_| {
            Fail::new(
                format!(
                    "The code wasn't entered within {}. Run siboot again for a new one.",
                    spoken(limits.code_entry)
                ),
                "code entry timed out",
            )
        })
}

/// "30 seconds", "10 minutes".
fn spoken(d: Duration) -> String {
    let secs = d.as_secs();
    let (n, unit) = if secs >= 60 && secs.is_multiple_of(60) {
        (secs / 60, "minute")
    } else {
        (secs, "second")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

fn parse_alt_irk(text: &str) -> Option<[u8; 16]> {
    let bytes = hex::decode(text).ok()?;
    bytes.try_into().ok()
}

// ---------------------------------------------------------------- rootshell

/// What came of rootshell's offer.
#[derive(Debug, PartialEq, Eq)]
enum Taken {
    /// The file, saved.
    File,
    /// Nothing is offering one.
    NotOffered,
    /// It was offering, and no code was typed.
    Cancelled,
}

/// siboot-pair's answer to one code.
enum Reply {
    File(Vec<u8>),
    WrongCode,
    Unexpected(String),
}

/// Send one code on `stream` and read siboot-pair's answer: `NO`, or
/// `OK <length>` and the file.
async fn exchange(stream: &mut TcpStream, code: &str) -> std::io::Result<Reply> {
    stream.write_all(format!("SIBOOT1 {code}\n").as_bytes()).await?;
    let mut reader = BufReader::new(stream);
    let mut answer = String::new();
    if reader.read_line(&mut answer).await? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "rootshell closed the connection without answering",
        ));
    }
    let answer = answer.trim();
    if answer == "NO" {
        return Ok(Reply::WrongCode);
    }
    let Some(len) = answer.strip_prefix("OK ").and_then(|n| n.parse::<usize>().ok()) else {
        return Ok(Reply::Unexpected(answer.to_string()));
    };
    if len == 0 || len > MAX_FILE {
        return Ok(Reply::Unexpected(answer.to_string()));
    }
    let mut file = vec![0u8; len];
    reader.read_exact(&mut file).await?;
    Ok(Reply::File(file))
}

/// Take the pairing file siboot-pair is offering at `offer`, for the transfer
/// code it shows, into `path`. The code proves the request comes from the
/// person at the iPhone, not another app. Each answer may take up to `wait`.
async fn take(
    offer: SocketAddr,
    path: &Path,
    mut ask: impl FnMut() -> Option<String>,
    wait: Duration,
) -> Result<Taken> {
    let connect = || timeout(Duration::from_secs(2), TcpStream::connect(offer));
    let Ok(Ok(mut stream)) = connect().await else {
        ui::detail(&format!("nothing is offering a pairing file on {offer}"));
        return Ok(Taken::NotOffered);
    };
    ui::ok("rootshell is offering its file");
    for attempt in 1..=CODE_TRIES {
        let Some(typed) = ask() else {
            return Ok(Taken::Cancelled);
        };
        let code: String = typed.chars().filter(char::is_ascii_digit).collect();
        ui::activity("sending the code to rootshell");
        // siboot-pair answers one request per connection.
        if attempt > 1 {
            stream = match connect().await {
                Ok(Ok(s)) => s,
                _ => {
                    return Err(Fail::new(
                        "rootshell stopped offering the pairing file. Run `./siboot-pair.wasm` in \
                         rootshell again, then siboot here.",
                        "",
                    ))
                }
            };
        }
        let file = match timeout(wait, exchange(&mut stream, &code)).await {
            Err(_) => {
                return Err(Fail::new(
                    "rootshell didn't answer: iOS has probably paused it. Check its Settings › \
                     Location Diary Mode is on Session Only, open rootshell for a moment, then \
                     come back here and run siboot again.",
                    format!("no answer on {offer} within {} s", wait.as_secs()),
                ))
            }
            Ok(Err(e)) => {
                return Err(Fail::new(
                    "The hand-over from rootshell broke off. Run siboot again.",
                    e.to_string(),
                ))
            }
            Ok(Ok(Reply::Unexpected(answer))) => {
                return Err(Fail::new("rootshell sent something that isn't a pairing file.", answer))
            }
            Ok(Ok(Reply::WrongCode)) => {
                let left = CODE_TRIES - attempt;
                if left > 0 {
                    ui::warn(&format!(
                        "That isn't the code rootshell shows ({left} more {}).",
                        if left == 1 { "try" } else { "tries" }
                    ));
                }
                continue;
            }
            Ok(Ok(Reply::File(file))) => file,
        };
        // Checked before it replaces a file that may still work.
        if let Err(e) = RpPairingFile::from_bytes(&file) {
            return Err(Fail::new("rootshell sent something that isn't a pairing file.", e.to_string()));
        }
        save(path, &file)?;
        ui::ok(&format!("took the pairing file from rootshell ({} bytes)", file.len()));
        return Ok(Taken::File);
    }
    Err(Fail::new(
        "The transfer code didn't match. Run siboot again and type the code rootshell shows.",
        "",
    ))
}

/// Write the pairing file, readable by this user only: it holds the private key.
fn save(path: &Path, file: &[u8]) -> Result<()> {
    let failed = |e: std::io::Error| Fail::new(format!("Couldn't save the pairing file at {}.", path.display()), e.to_string());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(failed)?;
    }
    std::fs::write(path, file).map_err(failed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(failed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_iphone;

    const QUICK: Duration = Duration::from_secs(5);

    const QUICK_LIMITS: Limits = Limits {
        device: Duration::from_secs(20),
        code_request: Duration::from_secs(20),
        code_entry: Duration::from_secs(20),
    };

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("siboot-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ------------------------------------------------------------ pairing here

    fn test_setup(
        bridge: Bridge,
        advertise: bool,
        pin_hook: Option<mpsc::UnboundedSender<String>>,
    ) -> HostSetup {
        HostSetup {
            bridge,
            advertise,
            name: "siboot test".into(),
            keepalive_dir: None,
            notify: false,
            limits: QUICK_LIMITS,
            pin_hook,
            player_check: Duration::from_millis(200),
        }
    }

    /// A whole pairing, the fake iPhone dialling the listener straight away,
    /// as Settings dials the advertised port.
    async fn pair_once(home: &Path, bridge: Option<Bridge>) -> (Paired, fake_iphone::Outcome) {
        let out = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (pin_tx, mut pin_rx) = mpsc::unbounded_channel();
        let advertise = bridge.is_some();
        let setup = test_setup(bridge.unwrap_or_default(), advertise, Some(pin_tx));
        let identity = Identity::load(&out, home).await;
        let host = run_host(listener, setup, identity, &out, home);
        let phone = fake_iphone::pair(addr, async move { pin_rx.recv().await.unwrap() });
        let (host, phone) = tokio::join!(host, phone);
        (host.expect("host paired"), phone.expect("phone paired"))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_iphone_pairs_and_the_file_holds_the_host_it_saw() {
        let home = temp_home("pair");
        let (paired, phone) = pair_once(&home, None).await;
        assert_eq!(paired.name, fake_iphone::NAME);
        assert_eq!(paired.model, fake_iphone::MODEL);

        let saved = RpPairingFile::read_from_file(home.join("rp_pairing_file.plist"))
            .await
            .unwrap();
        // The identity the iPhone was given in M6 is the one on disk…
        assert_eq!(saved.identifier, phone.host_identifier);
        assert_eq!(saved.public_key_bytes(), phone.host_public_key);
        // …the iPhone's altIRK came back in M5 and was kept…
        assert_eq!(saved.alt_irk.as_deref(), Some(&fake_iphone::ALT_IRK[..]));
        // …and the host's own altIRK was kept for the next run.
        assert_eq!(std::fs::read_to_string(home.join("host_alt_irk")).unwrap().len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(home.join("rp_pairing_file.plist"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_again_keeps_the_host_identity() {
        let home = temp_home("again");
        let (_, first) = pair_once(&home, None).await;
        let alt_irk = std::fs::read_to_string(home.join("host_alt_irk")).unwrap();
        let (_, second) = pair_once(&home, None).await;
        assert_eq!(first.host_identifier, second.host_identifier);
        assert_eq!(first.host_public_key, second.host_public_key);
        assert_eq!(std::fs::read_to_string(home.join("host_alt_irk")).unwrap(), alt_irk);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn through_minis_the_advert_matches_and_is_withdrawn() {
        let home = temp_home("bridge");
        let stub = fake_iphone::minis_stub(&home);
        let (_, phone) = pair_once(&home, Some(Bridge::in_dir(&stub))).await;

        let calls = std::fs::read_to_string(home.join("minis-calls.log")).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        let advertise = lines
            .iter()
            .find(|l| l.starts_with("minis-bonjour advertise"))
            .expect("advertised");
        let words: Vec<&str> = advertise.split(' ').collect();
        // advertise <type> <name> <port> key=value…
        assert_eq!(words[2], PAIRABLE_HOST_TYPE);
        assert_eq!(words[3], phone.host_identifier, "instance name is the identifier");
        for key in ["name", "identifier", "authTag", "model", "flags", "ver", "minVer"] {
            assert!(
                words.iter().any(|w| w.starts_with(&format!("{key}="))),
                "TXT {key} missing: {advertise}"
            );
        }
        assert!(words.contains(&format!("identifier={}", phone.host_identifier).as_str()));
        assert!(
            lines.iter().any(|l| l.starts_with("minis-bonjour stop adv-1")),
            "the advertisement wasn't withdrawn: {calls}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pairing_host_left_from_before_is_withdrawn_first() {
        let home = temp_home("leftover");
        let stub = fake_iphone::minis_stub(&home);
        std::fs::write(home.join("minis-leftover"), b"").unwrap();
        pair_once(&home, Some(Bridge::in_dir(&stub))).await;
        let calls = std::fs::read_to_string(home.join("minis-calls.log")).unwrap();
        let withdrawn = calls.find("minis-bonjour stop adv-0").expect("the leftover was withdrawn");
        let advertised = calls.find("minis-bonjour advertise").unwrap();
        assert!(withdrawn < advertised, "{calls}");
    }

    /// The first `advertise` is refused as iOS refuses it while its Local
    /// Network question is unanswered: siboot says what to allow, asks again,
    /// and pairs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_local_network_refusal_asks_again_and_pairs() {
        let home = temp_home("denied");
        let stub = fake_iphone::minis_stub(&home);
        std::fs::write(home.join("minis-deny-once"), b"").unwrap();
        pair_once(&home, Some(Bridge::in_dir(&stub))).await;
        let calls = std::fs::read_to_string(home.join("minis-calls.log")).unwrap();
        assert_eq!(calls.matches("minis-bonjour advertise").count(), 2, "{calls}");
        assert!(!home.join("minis-deny-once").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refusal_that_cant_be_fixed_in_settings_ends_the_run() {
        let home = temp_home("undeclared");
        let stub = fake_iphone::minis_stub(&home);
        std::fs::write(home.join("minis-undeclared"), b"").unwrap();
        let out = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let setup = test_setup(Bridge::in_dir(&stub), true, None);
        let identity = Identity::load(&out, &home).await;
        let failure = run_host(listener, setup, identity, &out, &home).await.err().unwrap();
        assert!(failure.advice.contains("can't advertise"), "{}", failure.advice);
        assert!(failure.advice.contains("1.16"), "{}", failure.advice);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_silence_plays_until_the_user_is_back_in_minis() {
        let home = temp_home("back");
        let stub = fake_iphone::minis_stub(&home);
        let marker = home.join("minis-background");
        std::fs::write(&marker, b"").unwrap();
        let out = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (pin_tx, mut pin_rx) = mpsc::unbounded_channel();
        let mut setup = test_setup(Bridge::in_dir(&stub), true, Some(pin_tx));
        setup.keepalive_dir = Some(home.clone());
        setup.notify = true;
        let identity = Identity::load(&out, &home).await;
        let host = run_host(listener, setup, identity, &out, &home);
        let log = home.join("minis-calls.log");
        let phone = async {
            let paired = fake_iphone::pair(addr, async move { pin_rx.recv().await.unwrap() }).await;
            // Still in Settings for a moment after typing the code.
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let calls = std::fs::read_to_string(&log).unwrap();
            assert!(
                !calls.contains("apple-player stop"),
                "the silence stopped with Minis in the background: {calls}"
            );
            std::fs::remove_file(&marker).unwrap();
            paired
        };
        let (host, phone) = tokio::join!(host, phone);
        host.expect("host paired");
        phone.expect("phone paired");
        let calls = std::fs::read_to_string(&log).unwrap();
        let played = calls.find("apple-player play").expect("the silence played");
        let code = calls.find("--title siboot pairing code").expect("the code was posted");
        let asked_back = calls.find("--title siboot paired this iPhone").expect("asked back to Minis");
        let stopped = calls.find("apple-player stop").expect("the silence was stopped");
        assert!(played < code && code < asked_back && asked_back < stopped, "{calls}");
        // The first notification went out while Minis was still in front.
        let ready = calls.find("--title Pair with siboot test is ready").expect("said it's ready");
        assert!(ready < code, "{calls}");
    }

    /// The player's sheet swiped away while siboot waits for Settings: it's
    /// opened again, and the pairing goes on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_closed_player_is_opened_again() {
        let home = temp_home("closed");
        let stub = fake_iphone::minis_stub(&home);
        let out = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (pin_tx, mut pin_rx) = mpsc::unbounded_channel();
        let mut setup = test_setup(Bridge::in_dir(&stub), true, Some(pin_tx));
        setup.keepalive_dir = Some(home.clone());
        let identity = Identity::load(&out, &home).await;
        let host = run_host(listener, setup, identity, &out, &home);
        let log = home.join("minis-calls.log");
        let plays = |log: &Path| {
            std::fs::read_to_string(log)
                .unwrap_or_default()
                .matches("apple-player play")
                .count()
        };
        let phone = async {
            while plays(&log) < 1 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            std::fs::write(home.join("minis-player-closed"), b"").unwrap();
            for _ in 0..100 {
                if plays(&log) >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            fake_iphone::pair(addr, async move { pin_rx.recv().await.unwrap() }).await
        };
        let (host, phone) = tokio::join!(host, phone);
        host.expect("host paired");
        phone.expect("phone paired");
        assert_eq!(plays(&log), 2, "{}", std::fs::read_to_string(&log).unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_wrong_code_gets_another_try() {
        let home = temp_home("wrongcode");
        let out = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (pin_tx, mut pin_rx) = mpsc::unbounded_channel();
        let setup = test_setup(Bridge::default(), false, Some(pin_tx));
        let identity = Identity::load(&out, &home).await;
        let host = run_host(listener, setup, identity, &out, &home);
        // A typo, then the entry tapped again and the new code typed.
        let phones = async move {
            let first = fake_iphone::pair(addr, async { "000000".to_string() }).await;
            let _ignored = pin_rx.recv().await.unwrap();
            let second = fake_iphone::pair(addr, async move { pin_rx.recv().await.unwrap() }).await;
            (first, second)
        };
        let (host, (first, second)) = tokio::join!(host, phones);
        assert!(first.is_err(), "the first phone typed a wrong code");
        second.expect("the second try paired");
        assert_eq!(host.expect("host paired on the second try").name, fake_iphone::NAME);
        assert!(out.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn three_wrong_codes_end_the_run_and_say_why() {
        let home = temp_home("wrong3");
        let out = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let setup = test_setup(Bridge::default(), false, None);
        let identity = Identity::load(&out, &home).await;
        let host = run_host(listener, setup, identity, &out, &home);
        let phones = async move {
            for _ in 0..MAX_ATTEMPTS {
                let paired = fake_iphone::pair(addr, async { "000000".to_string() }).await;
                assert!(paired.is_err(), "a wrong code paired");
            }
        };
        let (host, ()) = tokio::join!(host, phones);
        let failure = host.err().expect("host should give up");
        assert!(failure.advice.contains("code didn't match"), "{}", failure.advice);
        assert!(!out.exists(), "no pairing file for a failed pairing");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_connection_that_drops_before_the_code_doesnt_end_pairing() {
        let home = temp_home("stray");
        let out = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Queued ahead of the phone, and gone before saying anything.
        drop(std::net::TcpStream::connect(addr).unwrap());
        let (pin_tx, mut pin_rx) = mpsc::unbounded_channel();
        let setup = test_setup(Bridge::default(), false, Some(pin_tx));
        let identity = Identity::load(&out, &home).await;
        let host = run_host(listener, setup, identity, &out, &home);
        let phone = fake_iphone::pair(addr, async move { pin_rx.recv().await.unwrap() });
        let (host, phone) = tokio::join!(host, phone);
        assert_eq!(host.expect("host paired after the stray connection").name, fake_iphone::NAME);
        phone.expect("phone paired");
        assert!(out.exists());
    }

    #[tokio::test]
    async fn nobody_connecting_ends_with_advice() {
        let home = temp_home("idle");
        let out = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut setup = test_setup(Bridge::default(), false, None);
        setup.limits.device = Duration::from_millis(200);
        let identity = Identity::load(&out, &home).await;
        let failure = run_host(listener, setup, identity, &out, &home).await.err().unwrap();
        assert!(failure.advice.contains("Nobody tapped"), "{}", failure.advice);
    }

    #[tokio::test]
    async fn minis_without_its_bonjour_bridge_says_to_update() {
        let home = temp_home("old-minis");
        let empty = home.join("no-commands");
        std::fs::create_dir_all(&empty).unwrap();
        let why = can_pair_here(&Bridge::in_dir(&empty)).await.err().unwrap();
        assert!(why.contains("1.16"), "{why}");

        let stub = fake_iphone::minis_stub(&home);
        assert_eq!(can_pair_here(&Bridge::in_dir(&stub)).await, Ok(true));
        std::fs::write(home.join("minis-undeclared"), b"").unwrap();
        let why = can_pair_here(&Bridge::in_dir(&stub)).await.err().unwrap();
        assert!(why.contains("can't advertise"), "{why}");
    }

    #[test]
    fn words_and_irks() {
        assert_eq!(spoken(LIMITS.device), "10 minutes");
        assert_eq!(spoken(LIMITS.code_request), "30 seconds");
        assert_eq!(spoken(Duration::from_secs(60)), "1 minute");
        assert_eq!(parse_alt_irk("00112233445566778899aabbccddeeff").unwrap()[15], 0xff);
        assert!(parse_alt_irk("0011").is_none());
        assert!(parse_alt_irk("zz112233445566778899aabbccddeeff").is_none());
    }

    // ------------------------------------------------------------ rootshell

    fn a_pairing_file() -> Vec<u8> {
        RpPairingFile::generate("siboot test").to_bytes()
    }

    /// siboot-pair's side of the hand-off (rootshell/src/main.rs, `offer`):
    /// one request per connection, `NO` for a wrong code.
    async fn rootshell_offering(code: &'static str, file: Vec<u8>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut conn, _) = listener.accept().await.unwrap();
                let mut asked = String::new();
                BufReader::new(&mut conn).read_line(&mut asked).await.unwrap();
                if asked.trim() == format!("SIBOOT1 {code}") {
                    conn.write_all(format!("OK {}\n", file.len()).as_bytes()).await.unwrap();
                    conn.write_all(&file).await.unwrap();
                    return;
                }
                conn.write_all(b"NO\n").await.unwrap();
            }
        });
        addr
    }

    fn typing(codes: &'static [&'static str]) -> impl FnMut() -> Option<String> {
        let mut codes = codes.iter();
        move || codes.next().map(|c| c.to_string())
    }

    #[tokio::test]
    async fn the_file_comes_over_for_the_code_and_only_this_user_can_read_it() {
        let home = temp_home("take");
        let path = home.join("rp_pairing_file.plist");
        let file = a_pairing_file();
        let offer = rootshell_offering("481516", file.clone()).await;
        assert_eq!(take(offer, &path, typing(&["481 516"]), QUICK).await.unwrap(), Taken::File);
        assert_eq!(std::fs::read(&path).unwrap(), file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn a_mistyped_code_gets_another_try() {
        let home = temp_home("retry");
        let path = home.join("rp_pairing_file.plist");
        let offer = rootshell_offering("481516", a_pairing_file()).await;
        let taken = take(offer, &path, typing(&["481517", "481516"]), QUICK).await.unwrap();
        assert_eq!(taken, Taken::File);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn three_wrong_codes_say_so_and_keep_the_old_file() {
        let home = temp_home("wrong");
        let path = home.join("rp_pairing_file.plist");
        std::fs::write(&path, b"the old file").unwrap();
        let offer = rootshell_offering("481516", a_pairing_file()).await;
        let failure = take(offer, &path, typing(&["1", "2", "3"]), QUICK).await.err().unwrap();
        assert!(failure.advice.contains("didn't match"), "{}", failure.advice);
        assert_eq!(std::fs::read(&path).unwrap(), b"the old file");
    }

    #[tokio::test]
    async fn something_that_isnt_a_pairing_file_is_refused() {
        let home = temp_home("junk");
        let path = home.join("rp_pairing_file.plist");
        let offer = rootshell_offering("481516", b"not a plist".to_vec()).await;
        let failure = take(offer, &path, typing(&["481516"]), QUICK).await.err().unwrap();
        assert!(failure.advice.contains("isn't a pairing file"), "{}", failure.advice);
        assert!(!path.exists());
    }

    /// What siboot met on the iPhone with rootshell in the background: the
    /// connection goes through, and then nothing answers.
    #[tokio::test]
    async fn a_rootshell_that_never_answers_is_given_up_on() {
        let home = temp_home("silent");
        let path = home.join("rp_pairing_file.plist");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let offer = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let (conn, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(conn);
        });
        let started = std::time::Instant::now();
        let failure = take(offer, &path, typing(&["481516"]), Duration::from_millis(300))
            .await
            .err()
            .expect("a silent rootshell should end the wait");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(failure.advice.contains("didn't answer"), "{}", failure.advice);
        assert!(!path.exists());
        held.abort();
    }

    #[tokio::test]
    async fn nothing_offering_means_nothing_taken() {
        let home = temp_home("none");
        let path = home.join("rp_pairing_file.plist");
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap()
        };
        assert_eq!(take(closed, &path, typing(&[]), QUICK).await.unwrap(), Taken::NotOffered);
        assert!(!path.exists());
    }

    /// Cancel at rootshell's code: in Minis 1.16 that goes on to pairing here.
    #[tokio::test]
    async fn no_code_typed_is_a_cancel_not_a_failure() {
        let home = temp_home("cancel");
        let path = home.join("rp_pairing_file.plist");
        let offer = rootshell_offering("481516", a_pairing_file()).await;
        assert_eq!(take(offer, &path, typing(&[]), QUICK).await.unwrap(), Taken::Cancelled);
        assert!(!path.exists());
    }
}
