//! Step 1: is everything the later steps need actually here?
//!
//! Each check names what to change when it fails: Open Minis, which has to be
//! 1.16 or later, the internet, for sign-in and the download, and LocalDevVPN,
//! which carries everything to this iPhone. `siboot check` also says whether
//! this Open Minis can pair the iPhone.
//!
//! SideInstaller is downloaded here, before the iPhone is dialed
//! (download.rs): where GitHub is blocked, the VPN that reaches it is on until
//! then, and LocalDevVPN only from then on. When nothing answers at the
//! iPhone's address after that, siboot asks for LocalDevVPN and dials again.
//!
//! The iPhone's pairing service is nearly always on port 49152, but not
//! always. When it isn't there, this step finds the port it's on ([`reach_from`]),
//! and the later steps dial that one.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use idevice::remote_pairing::{RemotePairingClient, RpPairingSocket};
use idevice::IdeviceError;
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::bridge::{Bridge, BridgeError};
use crate::download::{self, Ipa};
use crate::fail::{chain, Fail, Result};
use crate::{mobile, ui, Args, Command};

/// Open Minis in the App Store app, for the page's button. The page hands
/// links that aren't http to iOS, so this opens the App Store itself.
pub const MINIS_APP_STORE: &str = "itms-apps://apps.apple.com/app/id6759188481";

/// What an Open Minis older than 1.16 is told.
pub const OLD_MINIS: &str = "This Open Minis is older than 1.16, and siboot needs Open Minis 1.16 \
     or later: it's the first that can pair this iPhone. Update Open Minis in the App Store, open \
     it again and run siboot.";

/// What step 1 found.
pub struct Checked {
    /// The internet was checked too.
    pub online: bool,
    /// Where the iPhone's pairing service answered: the address siboot was
    /// given, or the port the service turned out to be on.
    pub device: SocketAddr,
    /// With the internet checked: the IPA. A full run has it here for step 5;
    /// `siboot check` only looked.
    pub ipa: Option<Ipa>,
}

/// Asked for when nothing answers at the iPhone's address once SideInstaller
/// is here: no VPN carries it, or one that isn't LocalDevVPN, like one that
/// reaches GitHub where it's blocked.
const SWITCH_TITLE: &str = "Connect LocalDevVPN";
const SWITCH_STEPS: [&str; 2] = [
    "Open LocalDevVPN and connect it. iOS runs one VPN at a time, so it takes the place of any \
     other: siboot has SideInstaller already, and doesn't need GitHub again.",
    "Come back to Minis once LocalDevVPN shows Connected.",
];

pub async fn run(args: &Args) -> Result<Checked> {
    let in_minis = crate::web::available() || std::env::var_os("SIBOOT_MINIS_BIN").is_some();
    if in_minis {
        ui::activity("checking this Open Minis");
        check_minis(&Bridge::from_env()).await?;
        ui::ok("this Open Minis is 1.16 or later");
    }
    // The tunnel stays on this iPhone; sign-in and the download don't.
    let online = args.command == Command::All || args.command == Command::Check;
    let installs = args.command == Command::All;
    let ipa = if online {
        check_apple().await?;
        let mode = if installs { download::Mode::Get } else { download::Mode::Look };
        Some(download::get(args, mode).await?)
    } else {
        None
    };
    ui::activity(&format!("reaching this iPhone at {}", args.device));
    let mut first = probe(args.device).await;
    if let Err(Miss::Unreachable(f)) = &first {
        if installs && ui::interactive() {
            ui::detail(&format!("nothing answers at {}: {}", args.device, f.raw));
            ui::status("connect LocalDevVPN when you're ready");
            if !ui::get_ready(SWITCH_TITLE, &SWITCH_STEPS, &[], "Continue") {
                return Err(Fail::by_user("Stopped before LocalDevVPN was connected."));
            }
            ui::activity(&format!("reaching this iPhone at {}", args.device));
            first = probe(args.device).await;
        }
    }
    let (device, found) = match reach_from(args.device, first, DYNAMIC_PORTS, &KNOCKS).await {
        Ok(reached) => reached,
        // Refused on every port while the internet works: what an iPhone on
        // mobile data without Wi-Fi does, so Mobile Data Mode is offered
        // (mobile.rs). `check` only says so.
        Err(f) if f.refused
            && args.command != Command::Check
            && ui::interactive()
            && (online || mobile::online().await) =>
        {
            if !mobile::offer(args.device) {
                return Err(mobile::declined(f));
            }
            mobile::airplane_on(args.device, mobile::On::First).await?;
            ui::activity(&format!("reaching this iPhone at {}", args.device));
            reach_from(args.device, probe(args.device).await, DYNAMIC_PORTS, &KNOCKS)
                .await
                .map_err(mobile::not_mobile_data)?
        }
        Err(f) => {
            return Err(match &ipa {
                Some(ipa) if installs && ipa.github => with_kept_note(f, ipa),
                _ => f,
            })
        }
    };
    ui::ok(&format!(
        "this iPhone answers on {device} (RPPairing, wire protocol {})",
        found.wire_version
    ));
    if let Some(name) = found.name {
        ui::detail(&format!("it calls itself \"{name}\""));
    }
    if args.command == Command::Check {
        ui::activity("checking that Open Minis can pair this iPhone");
        match crate::pairing::can_pair_here(&crate::bridge::Bridge::from_env()).await {
            Ok(true) => ui::ok("Open Minis can pair this iPhone, and post the code as a notification"),
            Ok(false) => {
                ui::ok("Open Minis can pair this iPhone");
                ui::warn("Minis's notifications are off, so the pairing code will show only in Minis.");
            }
            Err(why) => ui::warn(&format!(
                "{why} The pairing then has to come from a file saved before: {}",
                crate::pairing::UPDATE_MINIS
            )),
        }
    }
    Ok(Checked { online, device, ipa })
}

/// A run that reached GitHub and stopped at the iPhone: SideInstaller is kept
/// now, so the next one, with LocalDevVPN, needn't reach GitHub.
fn with_kept_note(f: Fail, ipa: &Ipa) -> Fail {
    Fail {
        advice: format!(
            "{} {} is downloaded and kept, so the next run doesn't need GitHub.",
            f.advice,
            ipa.name()
        ),
        ..f
    }
}

/// Stop in an Open Minis older than 1.16. Minis doesn't tell the shell its
/// version, but pairing needs `minis-bonjour`, which came with 1.16 (App
/// Store, 2026-11-14): a Minis without it is older. Any other answer, a
/// refusal included, comes from 1.16 or later, and is step 2's to explain.
/// Only run inside Minis (`minis-open` on the PATH): elsewhere there's no
/// `minis-bonjour` either.
pub async fn check_minis(bridge: &Bridge) -> Result<()> {
    match bridge.bonjour_types().await {
        Err(BridgeError::Missing(_)) => {
            ui::link("Update Open Minis", MINIS_APP_STORE);
            Err(Fail::known(OLD_MINIS))
        }
        _ => Ok(()),
    }
}

/// Apple's sign-in server answers over TLS. Its `gsa.apple.com` chains to
/// Apple Root CA, which Alpine's bundle lacks, so it's trusted here as
/// isideload's GrandSlam client trusts it. GitHub's turn is download.rs's.
async fn check_apple() -> Result<()> {
    let apple_root = reqwest::Certificate::from_der(isideload::sideload::cert_identity::APPLE_ROOT)
        .map_err(|e| Fail::new("Couldn't load Apple's root certificate.", e.to_string()))?;
    let client = reqwest::Client::builder()
        .user_agent(format!("siboot/{}", crate::VERSION))
        .timeout(Duration::from_secs(20))
        .add_root_certificate(apple_root)
        .build()
        .map_err(|e| Fail::new("Couldn't start an HTTPS client.", e.to_string()))?;

    // Any HTTP answer proves DNS, the route and the certificate chain.
    ui::activity("reaching Apple's sign-in server");
    match client
        .get("https://gsa.apple.com/grandslam/GsService2/lookup")
        .send()
        .await
    {
        Ok(_) => ui::ok("Apple's sign-in server answers"),
        Err(e) => {
            return Err(Fail::new(
                "Couldn't reach Apple's sign-in server (gsa.apple.com). Check the internet \
                 connection.",
                crate::error_text::error_text(&e),
            ))
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct Found {
    pub wire_version: i64,
    pub name: Option<String>,
}

impl Found {
    fn from_handshake(reply: &plist::Value) -> Self {
        let wire_version = reply
            .as_dictionary()
            .and_then(|d| d.get("wireProtocolVersion"))
            .and_then(|v| v.as_signed_integer())
            .unwrap_or(0);
        let name = reply
            .as_dictionary()
            .and_then(|d| d.get("peerDeviceInfo"))
            .and_then(|v| v.as_dictionary())
            .and_then(|d| d.get("name"))
            .and_then(|v| v.as_string())
            .map(str::to_string);
        Self { wire_version, name }
    }
}

/// The ports iOS hands a listener that asks for any free one
/// (`net.inet.ip.portrange.first` to `.last`).
const DYNAMIC_PORTS: RangeInclusive<u16> = 49152..=65535;
/// lockdownd's: the one fixed port in that range, and not the pairing service.
const LOCKDOWN_PORT: u16 = 62078;

/// Dial the pairing service at `addr`. When LocalDevVPN carries the
/// connection but nothing there answers as the service, look for it on the
/// iPhone's other `ports`.
///
/// The service, remotepairingd's listener, takes whatever port iOS hands it
/// when it starts, and iOS hands TCP ports out in order from 49152, not at
/// random (`net.inet.tcp.randomize_ports` is 0, as on a Mac). Started first
/// after a restart, it gets 49152, which is why SideInstaller and StikDebug
/// dial that one blind. Started after something else, it gets a later one: on
/// 2026-11-18 one iPhone on a Wi-Fi advertised it (`_remotepairing._tcp`) on
/// 55021, while siboot users with Developer Mode on were refused at
/// 10.7.0.1:49152. Minis's shell can't browse Bonjour for that advert, so
/// siboot knocks on each port through LocalDevVPN instead. A closed port
/// refuses at once, so it takes seconds; from a Mac over Wi-Fi, 9 s.
///
/// `first` is how the dial of `addr` went: step 1 may have asked for
/// LocalDevVPN and dialed again in between.
async fn reach_from(
    addr: SocketAddr,
    first: std::result::Result<Found, Miss>,
    ports: impl IntoIterator<Item = u16>,
    knocks: &Knocks,
) -> Result<(SocketAddr, Found)> {
    let missed = match first {
        Ok(found) => return Ok((addr, found)),
        Err(Miss::Unreachable(f)) => return Err(f),
        Err(Miss::Elsewhere(f)) => f,
    };
    ui::detail(&format!("no pairing service at {addr}: {}", missed.raw));
    ui::status("looking for this iPhone's pairing service on its other ports");
    let ports = others(addr.port(), ports);
    let count = ports.len();
    let started = Instant::now();
    let search = search(addr.ip(), ports, knocks).await;
    ui::progress_end();
    let took = started.elapsed().as_secs_f32();
    match search {
        Search::Found(moved, found, knocked) => {
            ui::detail(&format!(
                "found it on port {} after {knocked} of {count} ports, in {took:.1} s",
                moved.port()
            ));
            Ok((moved, found))
        }
        Search::NotFound { open } => Err(Fail {
            raw: format!(
                "{}; no other port answered as it ({count} knocked on in {took:.1} s; took the \
                 connection: {})",
                missed.raw,
                listed(&open)
            ),
            ..missed
        }),
        Search::Stopped(f) => Err(f),
    }
}

/// `reach_from`, dialing `addr` first.
#[cfg(test)]
async fn reach(
    addr: SocketAddr,
    ports: impl IntoIterator<Item = u16>,
    knocks: &Knocks,
) -> Result<(SocketAddr, Found)> {
    reach_from(addr, probe(addr).await, ports, knocks).await
}

/// `ports` without the one already tried and lockdownd's, in order.
fn others(tried: u16, ports: impl IntoIterator<Item = u16>) -> Vec<u16> {
    ports
        .into_iter()
        .filter(|&p| p != tried && p != LOCKDOWN_PORT)
        .collect()
}

fn listed(ports: &[u16]) -> String {
    if ports.is_empty() {
        return "none".into();
    }
    let mut sorted = ports.to_vec();
    sorted.sort_unstable();
    sorted.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
}

/// Why the pairing service didn't answer at an address.
enum Miss {
    /// LocalDevVPN carried the connection, but the service isn't there: the
    /// port refused, or something else took it. It can be on another port.
    /// The advice is for when it isn't on any.
    Elsewhere(Fail),
    /// Nothing gets through to the iPhone.
    Unreachable(Fail),
}

/// Ask the iPhone's RPPairing listener for its handshake, as the first message
/// of a connection does. A reply proves the VPN route, Developer Mode and the
/// service all at once.
async fn probe(addr: SocketAddr) -> std::result::Result<Found, Miss> {
    let stream = match timeout(Duration::from_secs(8), TcpStream::connect(addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) if e.kind() == ErrorKind::ConnectionRefused => {
            return Err(Miss::Elsewhere(Fail {
                refused: true,
                ..Fail::new(
                    format!(
                        "The iPhone refused the connection at {addr}, and no other port on it \
                         answers as its pairing service. On mobile data without Wi-Fi, iOS \
                         refuses every port: `siboot` then offers Mobile Data Mode (beta). \
                         Otherwise turn on Developer Mode (Settings › Privacy & Security › \
                         Developer Mode), and if it's on, restart the iPhone, connect LocalDevVPN \
                         again and run siboot again."
                    ),
                    format!("{e} ({:?})", e.kind()),
                )
            }))
        }
        Ok(Err(e)) => return Err(Miss::Unreachable(unreachable(addr, &e))),
        Err(_) => {
            return Err(Miss::Unreachable(Fail::new(
                format!("Nothing answered at {addr}. Connect LocalDevVPN (it shows Connected)."),
                "connect timed out after 8 s",
            )))
        }
    };
    match timeout(Duration::from_secs(10), handshake(stream)).await {
        Ok(Ok(found)) => Ok(found),
        Ok(Err(e)) => Err(Miss::Elsewhere(Fail::new(
            format!(
                "Something at {addr} took the connection but didn't answer as an iPhone, and no \
                 other port there answers as its pairing service. Turn on Developer Mode \
                 (Settings › Privacy & Security › Developer Mode) and check LocalDevVPN is the \
                 VPN that's on."
            ),
            chain(&e),
        ))),
        Err(_) => Err(Miss::Elsewhere(Fail::new(
            format!(
                "The iPhone took the connection at {addr} but never answered, and no other port \
                 there answers as its pairing service. Restart LocalDevVPN."
            ),
            "handshake timed out after 10 s",
        ))),
    }
}

/// The handshake the pairing service answers first.
async fn handshake(stream: TcpStream) -> std::result::Result<Found, IdeviceError> {
    let mut client = RemotePairingClient::new(RpPairingSocket::new(stream), "siboot");
    let reply = client.attempt_pair_verify().await?;
    Ok(Found::from_handshake(&reply))
}

/// How to knock: kept apart so tests can wait less.
struct Knocks {
    /// Connections open at once. Minis's shell shares the app's file
    /// descriptors, so not many.
    width: usize,
    /// Through LocalDevVPN a closed port refuses and an open one accepts at
    /// once; this also covers one lost SYN, which iOS sends again after 1 s.
    connect: Duration,
    /// The pairing service answers its handshake within milliseconds.
    answer: Duration,
}

const KNOCKS: Knocks = Knocks {
    width: 32,
    connect: Duration::from_secs(2),
    answer: Duration::from_secs(3),
};

/// This many connections in a row that neither refuse nor connect: the route
/// has gone, and knocking on would take minutes.
const SILENT_IN_A_ROW: usize = 64;
/// More ports than an iPhone has open in its dynamic range took the
/// connection, so something else answers for this address. The iPhone
/// checked had two there besides the pairing service and lockdownd.
const MOST_OPEN: usize = 48;
/// Times the shell ran out of descriptors or buffers before siboot gives up.
const MOST_BUSY: usize = 256;

enum Search {
    /// Where it is, what it said, and how many ports were knocked on.
    Found(SocketAddr, Found, usize),
    /// No port answered as the pairing service. `open` took the connection.
    NotFound { open: Vec<u16> },
    /// Knocking had to stop, for the reason the advice says.
    Stopped(Fail),
}

/// Knock on each of `ports` at `ip`, `width` at a time, in order, until one
/// answers as the pairing service.
async fn search(ip: IpAddr, ports: Vec<u16>, knocks: &Knocks) -> Search {
    let total = ports.len().max(1);
    let mut queue = VecDeque::from(ports);
    let mut running = JoinSet::new();
    let mut width = knocks.width.max(1);
    let (mut settled, mut shown) = (0usize, None);
    let mut open = Vec::new();
    let (mut silent, mut busy) = (0usize, 0usize);
    loop {
        while running.len() < width {
            let Some(port) = queue.pop_front() else { break };
            running.spawn(knock(SocketAddr::new(ip, port), knocks.connect, knocks.answer));
        }
        // Leaving early drops the set, which aborts the knocks still out.
        let Some(joined) = running.join_next().await else {
            return Search::NotFound { open };
        };
        let Ok((port, outcome)) = joined else {
            continue;
        };
        match outcome {
            Knock::Answered(found) => {
                return Search::Found(SocketAddr::new(ip, port), found, settled + 1)
            }
            Knock::Closed => silent = 0,
            Knock::Open => {
                silent = 0;
                open.push(port);
                if open.len() > MOST_OPEN {
                    return Search::Stopped(Fail::new(
                        format!(
                            "Too many ports at {ip} take the connection for them to be this \
                             iPhone's: something other than LocalDevVPN answers there, like \
                             another VPN or a proxy. Turn it off, connect LocalDevVPN and run \
                             siboot again."
                        ),
                        format!(
                            "{} ports took the connection, none answered RPPairing: {}",
                            open.len(),
                            listed(&open)
                        ),
                    ));
                }
            }
            Knock::Silent => {
                silent += 1;
                if silent >= SILENT_IN_A_ROW {
                    return Search::Stopped(Fail::new(
                        "LocalDevVPN stopped carrying the connections while siboot looked for \
                         this iPhone's pairing service. Connect LocalDevVPN again (it shows \
                         Connected) and run siboot again.",
                        format!(
                            "{silent} connections in a row neither refused nor connected within \
                             {} s, the last to port {port}",
                            knocks.connect.as_secs_f32()
                        ),
                    ));
                }
            }
            Knock::Busy(e) => {
                busy += 1;
                if busy > MOST_BUSY {
                    return Search::Stopped(Fail::new(
                        "Open Minis couldn't open enough connections to look for this iPhone's \
                         pairing service. Close Minis, open it again and run siboot again.",
                        format!("{e} ({:?}), {busy} times", e.kind()),
                    ));
                }
                // Fewer at once, and this port again later.
                width = (width / 2).max(4).min(knocks.width.max(1));
                queue.push_back(port);
                continue;
            }
            Knock::Failed(e) => return Search::Stopped(unreachable(SocketAddr::new(ip, port), &e)),
        }
        settled += 1;
        let percent = (settled * 100 / total) as u64;
        if shown != Some(percent) {
            shown = Some(percent);
            ui::percent("ports checked", percent);
        }
    }
}

enum Knock {
    /// The pairing service.
    Answered(Found),
    /// Nothing listens there.
    Closed,
    /// Something took the connection, but didn't answer as the service.
    Open,
    /// Neither refused nor connected in time.
    Silent,
    /// Out of descriptors, buffers or local ports for now.
    Busy(std::io::Error),
    /// The route itself refused.
    Failed(std::io::Error),
}

async fn knock(addr: SocketAddr, connect: Duration, answer: Duration) -> (u16, Knock) {
    let outcome = match timeout(connect, TcpStream::connect(addr)).await {
        Err(_) => Knock::Silent,
        Ok(Err(e)) => match e.kind() {
            ErrorKind::TimedOut => Knock::Silent,
            ErrorKind::PermissionDenied
            | ErrorKind::NetworkUnreachable
            | ErrorKind::HostUnreachable
            | ErrorKind::NetworkDown => Knock::Failed(e),
            _ if is_busy(&e) => Knock::Busy(e),
            // Refused, reset, or anything else about this one port.
            _ => Knock::Closed,
        },
        Ok(Ok(stream)) => match timeout(answer, handshake(stream)).await {
            Ok(Ok(found)) => Knock::Answered(found),
            _ => Knock::Open,
        },
    };
    (addr.port(), outcome)
}

/// Worth trying again with fewer connections open.
fn is_busy(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::EMFILE | libc::ENFILE | libc::ENOBUFS | libc::EADDRNOTAVAIL | libc::EAGAIN)
    )
}

fn unreachable(addr: SocketAddr, e: &std::io::Error) -> Fail {
    let advice = match e.raw_os_error() {
        // EPERM: iOS refused the route itself, which is what happens with no VPN
        // carrying it (or with the iPhone's own Wi-Fi address as the target).
        Some(1) => format!(
            "iOS won't let Minis reach {addr}. Connect LocalDevVPN (it shows Connected) and use \
             its address, 10.7.0.1."
        ),
        _ => format!("Couldn't reach {addr}. Connect LocalDevVPN (it shows Connected)."),
    };
    Fail::new(advice, format!("{e} ({:?})", e.kind()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_iphone;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn an_open_minis_before_1_16_is_told_to_update() {
        let home = std::env::temp_dir().join(format!("siboot-test-minis-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let empty = home.join("no-commands");
        std::fs::create_dir_all(&empty).unwrap();

        let old = check_minis(&Bridge::in_dir(&empty)).await.err().unwrap();
        assert_eq!(old.advice, OLD_MINIS);
        assert!(old.known && !old.by_user && old.raw.is_empty());

        let stub = fake_iphone::minis_stub(&home);
        assert!(check_minis(&Bridge::in_dir(&stub)).await.is_ok());
        // 1.16 without the type declared is still 1.16: step 2 says so.
        std::fs::write(home.join("minis-undeclared"), b"").unwrap();
        assert!(check_minis(&Bridge::in_dir(&stub)).await.is_ok());
    }

    const QUICK: Knocks = Knocks {
        width: 32,
        connect: Duration::from_secs(2),
        answer: Duration::from_millis(300),
    };

    async fn listener() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    /// The iPhone's pairing service: each connection's handshake answered as
    /// the iPhone answers it.
    async fn pairing_service() -> u16 {
        let (listener, port) = listener().await;
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut head = [0u8; 11];
                    if s.read_exact(&mut head).await.is_err() || &head[..9] != b"RPPairing" {
                        return;
                    }
                    let mut body = vec![0; u16::from_be_bytes([head[9], head[10]]) as usize];
                    if s.read_exact(&mut body).await.is_err() {
                        return;
                    }
                    let reply = serde_json::json!({
                        "message": {"plain": {"_0": {"response": {"_1": {"handshake": {"_0": {
                            "deviceOptions": {"allowsPairSetup": true},
                            "peerDeviceInfo": {"name": "Test iPhone"},
                            "wireProtocolVersion": 26
                        }}}}}}},
                        "originatedBy": "device",
                        "sequenceNumber": 0
                    })
                    .to_string();
                    let mut frame = b"RPPairing".to_vec();
                    frame.extend_from_slice(&(reply.len() as u16).to_be_bytes());
                    frame.extend_from_slice(reply.as_bytes());
                    let _ = s.write_all(&frame).await;
                    let _ = s.read(&mut [0u8; 64]).await;
                });
            }
        });
        port
    }

    /// Takes the connection and says nothing, as the iPhone's other
    /// listeners do.
    async fn silent_service() -> u16 {
        let (listener, port) = listener().await;
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((s, _)) = listener.accept().await {
                held.push(s);
            }
        });
        port
    }

    /// Answers, but not as the pairing service.
    async fn other_service() -> u16 {
        let (listener, port) = listener().await;
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            }
        });
        port
    }

    /// A port nothing listens on any more.
    async fn closed_port() -> u16 {
        listener().await.1
    }

    fn local(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[tokio::test]
    async fn a_refused_port_sends_siboot_looking_for_the_service() {
        let (first, closed) = (closed_port().await, closed_port().await);
        let (silent, other, service) =
            (silent_service().await, other_service().await, pairing_service().await);
        let mut ports = vec![closed; 200];
        ports.extend([silent, other, closed, service]);

        let (at, found) = reach(local(first), ports, &QUICK).await.unwrap();
        assert_eq!(at, local(service));
        assert_eq!(found.wire_version, 26);
        assert_eq!(found.name.as_deref(), Some("Test iPhone"));
    }

    #[tokio::test]
    async fn so_does_something_else_on_that_port() {
        let (other, service) = (other_service().await, pairing_service().await);
        let (at, _) = reach(local(other), [other, service], &QUICK).await.unwrap();
        assert_eq!(at, local(service));
    }

    #[tokio::test]
    async fn the_service_where_it_should_be_is_taken_without_looking() {
        let service = pairing_service().await;
        // Nothing to look through: the first answer is the one used.
        let (at, found) = reach(local(service), std::iter::empty(), &QUICK).await.unwrap();
        assert_eq!((at, found.wire_version), (local(service), 26));
    }

    #[tokio::test]
    async fn nowhere_is_said_with_what_took_the_connection() {
        let first = closed_port().await;
        let (silent, other, closed) =
            (silent_service().await, other_service().await, closed_port().await);
        let f = reach(local(first), [closed, other, silent], &QUICK).await.unwrap_err();
        assert!(f.advice.starts_with(&format!(
            "The iPhone refused the connection at 127.0.0.1:{first}, and no other port"
        )));
        assert!(f.advice.contains("restart the iPhone"));
        assert!(f.raw.contains("Connection refused"), "{}", f.raw);
        let (low, high) = (other.min(silent), other.max(silent));
        assert!(f.raw.contains("3 knocked on"), "{}", f.raw);
        assert!(f.raw.ends_with(&format!("took the connection: {low}, {high})")), "{}", f.raw);
        assert!(!f.known && !f.by_user);
    }

    #[tokio::test]
    async fn ports_that_all_take_the_connection_are_not_this_iphones() {
        let (first, other) = (closed_port().await, other_service().await);
        let f = reach(local(first), vec![other; MOST_OPEN + 5], &QUICK)
            .await
            .unwrap_err();
        assert!(f.advice.contains("another VPN or a proxy"), "{}", f.advice);
        assert!(f.raw.starts_with(&format!("{} ports took the connection", MOST_OPEN + 1)));
    }

    #[test]
    fn lockdownd_and_the_port_tried_are_not_knocked_on_again() {
        let ports = others(49152, DYNAMIC_PORTS);
        assert_eq!(ports.len(), 65535 - 49152 + 1 - 2);
        assert_eq!((ports[0], *ports.last().unwrap()), (49153, 65535));
        assert!(!ports.contains(&LOCKDOWN_PORT) && !ports.contains(&49152));
        // Another port given with --port: 49152 is looked at first.
        assert_eq!(others(50000, DYNAMIC_PORTS)[0], 49152);
        assert_eq!(listed(&[54666, 49999]), "49999, 54666");
        assert_eq!(listed(&[]), "none");
    }
}
