//! Mobile Data Mode (beta): siboot on mobile data, without Wi-Fi.
//!
//! On mobile data alone, iOS keeps the iPhone's remote pairing listener
//! closed, so every connection LocalDevVPN carries to 10.7.0.1 is refused, on
//! every port. In Airplane Mode, with LocalDevVPN still connected, the
//! listener is open again, and a tunnel opened then keeps working once mobile
//! data is back (measured with SideInstaller on an iPhone 16, iOS 27.2.1, on
//! 2026-11-26; SideInstaller's Mobile Data Mode is the same idea). So siboot
//! has the user turn Airplane Mode on while it connects to the iPhone, and off
//! again while it needs the internet (sign-in and signing), and keeps the
//! tunnel it opened rather than opening another to install.
//!
//! Open Minis's shell can't see which network the iPhone is on: its netlink
//! answers an empty dump, so there's no list of interfaces to read. siboot
//! goes by what it can see instead. The iPhone refusing every port while
//! Apple's sign-in server answers is when it offers the mode; the iPhone
//! taking a connection is how it knows Airplane Mode is on; Apple's server
//! answering again is how it knows Airplane Mode is off.

use std::future::Future;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::fail::{Fail, Result};
use crate::{tunnel, ui};

/// On from the user's yes to the end of the run. While it is, a refused dial
/// asks for Airplane Mode again instead of failing, and the tunnel step 3
/// opens is kept for step 6.
static ON: AtomicBool = AtomicBool::new(false);
/// How often the run asked for Airplane Mode on, and off.
static ASKED_ON: AtomicUsize = AtomicUsize::new(0);
static ASKED_OFF: AtomicUsize = AtomicUsize::new(0);
/// What became of the tunnel kept through sign-in and signing: 0 not tried,
/// 1 it still answered, 2 it had closed.
static KEPT: AtomicU8 = AtomicU8::new(0);

pub fn is_on() -> bool {
    ON.load(Ordering::Relaxed)
}

const OFFER_TITLE: &str = "Try Mobile Data Mode (beta)?";
const CONTROL_CENTER: &str = "Open Control Center: swipe down from the top-right corner of the screen.";
const COME_BACK: &str = "Come back to Minis. siboot carries on by itself.";

/// Asks whether to try the mode, once the iPhone at `device` refused every
/// port while the internet works. True to go ahead, and the mode is on.
pub fn offer(device: SocketAddr) -> bool {
    ui::status("this iPhone refuses siboot: try Mobile Data Mode?");
    let notes = [
        format!(
            "This iPhone refused siboot at {} and on every other port. If it's on mobile data, \
             not Wi-Fi, that's why: iOS doesn't accept new connections from LocalDevVPN then.",
            device.ip()
        ),
        "Mobile Data Mode works around that with Airplane Mode: you turn it on while siboot \
         connects to this iPhone, and off again when siboot needs the internet. siboot tells \
         you when."
            .to_string(),
        "Wi-Fi is the stable way, and the safer one. To use it, turn on Wi-Fi and run siboot \
         again."
            .to_string(),
    ];
    let notes: Vec<&str> = notes.iter().map(String::as_str).collect();
    let yes = ui::get_ready(OFFER_TITLE, &[], &notes, "Try Mobile Data Mode");
    if yes {
        ON.store(true, Ordering::Relaxed);
        ui::note("Mobile Data Mode (beta) is on for this run.");
    }
    yes
}

/// The run stops where the user would rather use Wi-Fi. `refused` is what
/// the iPhone said, for the log.
pub fn declined(refused: Fail) -> Fail {
    Fail {
        advice: "Stopped, to use Wi-Fi instead: turn it on, then run siboot again. If this iPhone \
                 is on Wi-Fi already, turn on Developer Mode (Settings › Privacy & Security › \
                 Developer Mode), and if it's on, restart the iPhone, connect LocalDevVPN again \
                 and run siboot again."
            .into(),
        by_user: true,
        ..refused
    }
}

/// The iPhone refused in Airplane Mode too: mobile data isn't what stops it.
pub fn not_mobile_data(f: Fail) -> Fail {
    if !f.refused {
        return f;
    }
    Fail {
        advice: "This iPhone refuses siboot in Airplane Mode too, so mobile data isn't what stops \
                 it. Turn on Developer Mode (Settings › Privacy & Security › Developer Mode), and \
                 if it's on, restart the iPhone, connect LocalDevVPN again and run siboot again."
            .into(),
        ..f
    }
}

/// Why Airplane Mode has to come on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    /// The first connection of the run.
    First,
    /// A new connection, after the one siboot had closed or Airplane Mode
    /// went off early.
    Again,
}

/// What the wait for Airplane Mode saw.
#[derive(Debug, PartialEq, Eq)]
pub enum Seen {
    /// The iPhone took a connection.
    Answers,
    /// The internet went (Airplane Mode is on), but the iPhone still
    /// refuses: its pairing service is on another port, or not there at all.
    OfflineRefusing,
}

/// How the waits go: kept apart so tests can wait less.
pub struct Waits {
    /// Between dials of the iPhone.
    pub dial_every: Duration,
    /// How long one dial may take.
    pub dial: Duration,
    /// Between checks of the internet.
    pub online_every: Duration,
    /// How long the iPhone may go on refusing once the internet has gone.
    pub refusing_offline: Duration,
    /// How long siboot waits for the user at most.
    pub longest: Duration,
}

pub const WAITS: Waits = Waits {
    dial_every: Duration::from_secs(1),
    dial: Duration::from_secs(3),
    online_every: Duration::from_secs(2),
    refusing_offline: Duration::from_secs(8),
    longest: Duration::from_secs(10 * 60),
};

/// Has the user turn Airplane Mode on, and waits until the iPhone takes a
/// connection at `addr`, or until Airplane Mode is plainly on while `addr`
/// still refuses. The caller dials again either way.
pub async fn airplane_on(addr: SocketAddr, why: On) -> Result<Seen> {
    airplane_on_with(addr, why, &WAITS, online).await
}

pub async fn airplane_on_with<F, Fut>(addr: SocketAddr, why: On, waits: &Waits, online: F) -> Result<Seen>
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    ASKED_ON.fetch_add(1, Ordering::Relaxed);
    let notes: &[&str] = match why {
        On::First => &[],
        On::Again => &["siboot needs a new connection to this iPhone, and on mobile data it can only \
                        make one in Airplane Mode."],
    };
    ui::show_guide(
        "Turn on Airplane Mode",
        &[
            CONTROL_CENTER,
            "Tap Airplane Mode to turn it on. Leave Wi-Fi off and LocalDevVPN connected.",
            COME_BACK,
        ],
        notes,
    );
    ui::status("waiting for Airplane Mode");
    let started = Instant::now();
    let mut checked: Option<Instant> = None;
    let mut offline_since: Option<Instant> = None;
    let mut vpn_said = false;
    let seen = loop {
        match timeout(waits.dial, TcpStream::connect(addr)).await {
            Ok(Ok(_)) => break Ok(Seen::Answers),
            Ok(Err(e)) if e.kind() == ErrorKind::ConnectionRefused => {}
            other => {
                // Not carried at all: LocalDevVPN reconnects when the network
                // changes, and says so if it can't.
                if !vpn_said {
                    vpn_said = true;
                    ui::status("waiting for Airplane Mode, and for LocalDevVPN to show Connected");
                    ui::detail(&match other {
                        Ok(Err(e)) => format!("dialing {addr}: {e}"),
                        _ => format!("dialing {addr}: no answer within {} s", waits.dial.as_secs()),
                    });
                }
            }
        }
        if checked.is_none_or(|at| at.elapsed() >= waits.online_every) {
            checked = Some(Instant::now());
            if online().await {
                offline_since = None;
            } else {
                offline_since.get_or_insert_with(Instant::now);
            }
        }
        if offline_since.is_some_and(|since| since.elapsed() >= waits.refusing_offline) {
            break Ok(Seen::OfflineRefusing);
        }
        if started.elapsed() >= waits.longest {
            break Err(Fail::new(
                format!(
                    "Airplane Mode didn't come on within {} minutes, so siboot stopped. Turn it \
                     off to get mobile data back, and run siboot again when you're ready.",
                    waits.longest.as_secs() / 60
                ),
                format!("{addr} still refused; Apple's sign-in server still answered"),
            ));
        }
        tokio::time::sleep(waits.dial_every).await;
    };
    ui::guide_gone();
    match &seen {
        Ok(Seen::Answers) => ui::ok("Airplane Mode is on, and this iPhone takes the connection"),
        Ok(Seen::OfflineRefusing) => {
            ui::detail(&format!("the internet has gone, but {addr} still refuses"))
        }
        Err(_) => {}
    }
    seen
}

/// What needs the internet next, said while asking for Airplane Mode off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    SignIn,
    Signing,
}

impl Need {
    fn sentence(self) -> &'static str {
        match self {
            Need::SignIn => {
                "siboot signs in to your Apple Account next, which needs the internet. Its \
                 connection to this iPhone stays open meanwhile."
            }
            Need::Signing => "siboot signs SideInstaller next, which needs the internet.",
        }
    }
}

/// In Mobile Data Mode, before what needs the internet: when Airplane Mode
/// is still on, has the user turn it off and waits for the internet.
pub async fn require_internet(need: Need) -> Result<()> {
    require_internet_with(need, is_on(), &WAITS, online).await
}

/// `require_internet`, with whether the mode is `on` given.
pub async fn require_internet_with<F, Fut>(need: Need, on: bool, waits: &Waits, online: F) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    if !on || online().await {
        return Ok(());
    }
    ASKED_OFF.fetch_add(1, Ordering::Relaxed);
    ui::show_guide(
        "Turn off Airplane Mode",
        &[
            CONTROL_CENTER,
            "Tap Airplane Mode to turn it off, so mobile data comes back. Leave LocalDevVPN \
             connected.",
            COME_BACK,
        ],
        &[need.sentence()],
    );
    ui::status("waiting for the internet");
    let started = Instant::now();
    let outcome = loop {
        tokio::time::sleep(waits.online_every).await;
        if online().await {
            break Ok(());
        }
        if started.elapsed() >= waits.longest {
            break Err(Fail::new(
                format!(
                    "The internet didn't come back within {} minutes. Turn Airplane Mode off \
                     and check mobile data is on, then run siboot again.",
                    waits.longest.as_secs() / 60
                ),
                "Apple's sign-in server never answered",
            ));
        }
    };
    ui::guide_gone();
    if outcome.is_ok() {
        ui::ok("Airplane Mode is off, and Apple's sign-in server answers");
    }
    outcome
}

/// Step 6 in Mobile Data Mode: the tunnel kept through sign-in and signing,
/// while it still answers. None when a new one has to be opened.
pub async fn reuse(kept: Option<tunnel::Connection>) -> Option<tunnel::Connection> {
    let mut kept = kept?;
    ui::activity("checking that the tunnel kept from step 3 still answers");
    match kept.answers().await {
        Ok(()) => {
            KEPT.store(1, Ordering::Relaxed);
            ui::ok("the tunnel opened in Airplane Mode still answers, so siboot installs through it");
            Some(kept)
        }
        Err(raw) => {
            KEPT.store(2, Ordering::Relaxed);
            ui::note("The tunnel siboot kept has closed, so it opens a new one.");
            ui::detail(&raw);
            None
        }
    }
}

/// Said with the outcome of a restart that leaves Airplane Mode on.
pub fn restart_note() -> Option<&'static str> {
    is_on().then_some("Airplane Mode stays on through the restart: turn it off once the iPhone is back.")
}

/// At the end of a run in the mode: the mode's numbers go into the log, and
/// the user is told to turn Airplane Mode off while it's still on.
pub async fn wrap_up() {
    if !is_on() {
        return;
    }
    let kept = match KEPT.load(Ordering::Relaxed) {
        1 => "still answered at step 6",
        2 => "had closed by step 6",
        _ => "wasn't tried",
    };
    ui::record(&format!(
        "      Mobile Data Mode: Airplane Mode asked on {} time(s) and off {} time(s); the tunnel \
         kept through sign-in {kept}",
        ASKED_ON.load(Ordering::Relaxed),
        ASKED_OFF.load(Ordering::Relaxed)
    ));
    if !online().await {
        ui::note("Airplane Mode is still on. Turn it off to get mobile data back: siboot is done with this iPhone.");
    }
}

/// Whether Apple's sign-in server takes a connection: the internet as siboot
/// needs it. Quick to fail in Airplane Mode, with no route and no DNS.
pub async fn online() -> bool {
    matches!(
        timeout(Duration::from_secs(4), TcpStream::connect(("gsa.apple.com", 443))).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    const QUICK: Waits = Waits {
        dial_every: Duration::from_millis(20),
        dial: Duration::from_millis(200),
        online_every: Duration::from_millis(20),
        refusing_offline: Duration::from_millis(150),
        longest: Duration::from_secs(5),
    };

    /// A port nothing listens on: it refuses.
    async fn closed_port() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    }

    #[tokio::test]
    async fn airplane_mode_is_on_once_the_iphone_takes_the_connection() {
        let addr = closed_port().await;
        // The listener comes up a little later, as remotepairingd's does once
        // Airplane Mode is on.
        let opened = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let listener = TcpListener::bind(addr).await.unwrap();
            let _ = listener.accept().await;
        });
        let seen = airplane_on_with(addr, On::First, &QUICK, || async { true }).await;
        assert_eq!(seen.unwrap(), Seen::Answers);
        opened.await.unwrap();
    }

    #[tokio::test]
    async fn offline_and_still_refusing_hands_back_to_look_elsewhere() {
        let addr = closed_port().await;
        let seen = airplane_on_with(addr, On::Again, &QUICK, || async { false }).await;
        assert_eq!(seen.unwrap(), Seen::OfflineRefusing);
    }

    #[tokio::test]
    async fn the_wait_for_airplane_mode_ends() {
        let addr = closed_port().await;
        let waits = Waits {
            longest: Duration::from_millis(200),
            ..QUICK
        };
        let f = airplane_on_with(addr, On::First, &waits, || async { true })
            .await
            .unwrap_err();
        assert!(f.advice.starts_with("Airplane Mode didn't come on"), "{}", f.advice);
    }

    #[tokio::test]
    async fn the_internet_is_waited_for_only_in_the_mode_and_offline() {
        // Outside the mode nothing is asked, whatever the network.
        require_internet_with(Need::SignIn, false, &QUICK, || async { false })
            .await
            .unwrap();
        // Back after a few checks, as mobile data is once Airplane Mode is off.
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        require_internet_with(Need::Signing, true, &QUICK, move || {
            let n = counted.fetch_add(1, Ordering::Relaxed);
            async move { n >= 3 }
        })
        .await
        .unwrap();
        assert!(calls.load(Ordering::Relaxed) >= 4);
    }

    #[tokio::test]
    async fn the_wait_for_the_internet_ends() {
        let waits = Waits {
            longest: Duration::from_millis(100),
            ..QUICK
        };
        let asked = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&asked);
        let f = require_internet_with(Need::SignIn, true, &waits, move || {
            seen.store(true, Ordering::Relaxed);
            async { false }
        })
        .await
        .unwrap_err();
        assert!(asked.load(Ordering::Relaxed));
        assert!(f.advice.starts_with("The internet didn't come back"), "{}", f.advice);
    }

    #[test]
    fn a_refusal_in_airplane_mode_says_mobile_data_isnt_it() {
        let refused = Fail {
            refused: true,
            ..Fail::new("The iPhone refused the connection.", "Connection refused")
        };
        let f = not_mobile_data(refused);
        assert!(f.advice.starts_with("This iPhone refuses siboot in Airplane Mode too"));
        assert!(f.refused);
        let other = not_mobile_data(Fail::new("Nothing answered.", "timed out"));
        assert_eq!(other.advice, "Nothing answered.");
    }

    #[test]
    fn declining_stops_as_the_users_choice_and_keeps_the_cause() {
        let refused = Fail {
            refused: true,
            ..Fail::new("The iPhone refused the connection.", "Connection refused (os error 61)")
        };
        let f = declined(refused);
        assert!(f.by_user);
        assert!(f.advice.starts_with("Stopped, to use Wi-Fi instead"));
        assert_eq!(f.raw, "Connection refused (os error 61)");
    }
}
