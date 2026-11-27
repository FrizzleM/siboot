//! Step 3: the tunnel, opened the way SideInstaller opens it
//! (`tunnel_create_rppairing_multihost` in ../SideInstaller/rust-core/vendor/idevice-ffi):
//!
//! ```text
//! 10.7.0.1:49152 (RPPairing) → pair-verify → createListener
//!   → TLS-PSK → CDTunnel → software TCP stack → RSD → services
//! ```
//!
//! LocalDevVPN sends 10.7.0.1 back to this iPhone, and the iPhone opens the
//! tunnel listener on the same address, so there is one host to dial.

use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use idevice::lockdown::LockdownClient;
use idevice::remote_pairing::{
    connect_tls_psk_tunnel_native, RemotePairingClient, RpPairingFile, RpPairingSocket,
};
use idevice::rsd::RsdHandshake;
use idevice::tcp::adapter::Adapter;
use idevice::tcp::handle::AdapterHandle;
use idevice::RsdService;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::fail::{chain, Fail, Result};
use crate::{mobile, ui};

/// The name this host gives in its messages to the device.
const HOSTNAME: &str = "siboot";

/// A tunnel with its RSD handshake: what every service connects through.
pub struct Connection {
    pub adapter: AdapterHandle,
    pub handshake: RsdHandshake,
}

pub struct DeviceInfo {
    pub name: String,
    pub product_type: String,
    pub ios_version: String,
    pub udid: String,
}

/// Pair-verify with the file at `path`, and nothing more: does this iPhone
/// still trust it?
pub async fn verify(addr: SocketAddr, path: &Path) -> Result<()> {
    let mut file = read_pairing(path).await?;
    pair_verified(addr, &mut file).await.map(|_| ())
}

/// The whole tunnel.
pub async fn open(addr: SocketAddr, path: &Path) -> Result<Connection> {
    let mut file = read_pairing(path).await?;
    let mut rpc = pair_verified(addr, &mut file).await?;

    let port = step(
        "asking the iPhone for a tunnel listener",
        15,
        rpc.create_tcp_listener(),
    )
    .await?;
    ui::detail(&format!("tunnel listener on port {port}"));
    ui::activity("connecting to the tunnel listener");
    let stream = match timeout(
        Duration::from_secs(10),
        TcpStream::connect((addr.ip(), port)),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return Err(Fail::new(
                format!(
                    "The iPhone opened a tunnel port but {}:{port} can't be reached.",
                    addr.ip()
                ),
                e.to_string(),
            ))
        }
        Err(_) => {
            return Err(Fail::new(
                format!(
                    "The iPhone opened a tunnel port but {}:{port} didn't answer.",
                    addr.ip()
                ),
                "connect timed out",
            ))
        }
    };
    let tunnel = step(
        "starting the encrypted tunnel",
        20,
        connect_tls_psk_tunnel_native(stream, rpc.encryption_key()),
    )
    .await?;
    let client_ip: IpAddr = tunnel.info.client_address.parse().map_err(|e| {
        Fail::new(
            "The tunnel reported an address siboot can't read.",
            format!("{e}"),
        )
    })?;
    let server_ip: IpAddr = tunnel.info.server_address.parse().map_err(|e| {
        Fail::new(
            "The tunnel reported an address siboot can't read.",
            format!("{e}"),
        )
    })?;
    let mtu = tunnel.info.mtu as usize;
    let rsd_port = tunnel.info.server_rsd_port;
    ui::detail(&format!(
        "tunnel up: {client_ip} → {server_ip}, MTU {mtu}, RSD on {rsd_port}"
    ));

    let mut adapter = Adapter::new(Box::new(tunnel.into_inner()), client_ip, server_ip);
    // Capped at 8192 as idevice (edf2587) and SideInstaller cap it: the
    // tunnel's MTU can be far larger than the segments the iPhone's end takes.
    adapter.set_mss(mtu.min(8192).saturating_sub(60));
    let mut adapter = adapter.to_async_handle();
    ui::activity("reaching RSD inside the tunnel");
    let rsd = match timeout(Duration::from_secs(15), adapter.connect(rsd_port)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return Err(Fail::new(
                "The tunnel came up, but RSD inside it didn't answer.",
                e.to_string(),
            ))
        }
        Err(_) => {
            return Err(Fail::new(
                "The tunnel came up, but RSD inside it didn't answer.",
                "timed out",
            ))
        }
    };
    let handshake = step("reading the iPhone's services", 15, RsdHandshake::new(rsd)).await?;
    ui::detail(&format!(
        "RSD: {} services, protocol {}",
        handshake.services.len(),
        handshake.protocol_version
    ));
    Ok(Connection { adapter, handshake })
}

impl Connection {
    /// Whether the tunnel still answers: lockdownd inside it gives the iOS
    /// version within a few seconds. The cause when it doesn't.
    pub async fn answers(&mut self) -> std::result::Result<(), String> {
        let asked = async {
            let mut lockdown = LockdownClient::connect_rsd(&mut self.adapter, &mut self.handshake).await?;
            lockdown.get_value(Some("ProductVersion"), None).await
        };
        match timeout(Duration::from_secs(8), asked).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(chain(&e)),
            Err(_) => Err("lockdownd didn't answer within 8 s".into()),
        }
    }

    /// Name, model, iOS version and UDID, from lockdownd inside the tunnel.
    pub async fn describe(&mut self) -> Result<DeviceInfo> {
        let mut lockdown = step(
            "asking lockdownd about this iPhone",
            15,
            LockdownClient::connect_rsd(&mut self.adapter, &mut self.handshake),
        )
        .await?;
        let values = step(
            "asking lockdownd about this iPhone",
            15,
            lockdown.get_value(None, None),
        )
        .await?;
        let field = |key: &str| {
            values
                .as_dictionary()
                .and_then(|d| d.get(key))
                .and_then(|v| v.as_string())
                .map(str::to_string)
        };
        // RSD's own properties carry the UDID too, if lockdownd leaves it out.
        let udid = field("UniqueDeviceID").or_else(|| {
            self.handshake
                .properties
                .get("UniqueDeviceID")
                .and_then(|v| v.as_string())
                .map(str::to_string)
        });
        let udid = udid.ok_or_else(|| {
            Fail::new(
                "This iPhone didn't say its UDID, which signing needs.",
                "no UniqueDeviceID from lockdownd or RSD",
            )
        })?;
        Ok(DeviceInfo {
            name: field("DeviceName").unwrap_or_else(|| "iPhone".into()),
            product_type: field("ProductType").unwrap_or_else(|| "iPhone".into()),
            ios_version: field("ProductVersion").unwrap_or_else(|| "?".into()),
            udid,
        })
    }
}

async fn read_pairing(path: &Path) -> Result<RpPairingFile> {
    RpPairingFile::read_from_file(path).await.map_err(|e| {
        Fail::new(
            format!(
                "Couldn't read the pairing file at {}. Run siboot again, and it pairs this iPhone again.",
                path.display()
            ),
            chain(&e),
        )
    })
}

/// Connect to the RPPairing listener and pair-verify, without the fallback to
/// pair-setup that `RemotePairingClient::connect` takes: the device refuses a
/// computer-initiated pair-setup, so it would only hang up.
async fn pair_verified(
    addr: SocketAddr,
    file: &mut RpPairingFile,
) -> Result<RemotePairingClient<RpPairingSocket<TcpStream>>> {
    let stream = match dial(addr).await {
        Ok(s) => s,
        // On mobile data the iPhone refuses again once Airplane Mode is off:
        // in Mobile Data Mode that asks for it again rather than stopping.
        Err(e) if e.kind() == ErrorKind::ConnectionRefused && mobile::is_on() => {
            ui::detail(&format!("{addr} refused: {e}"));
            mobile::airplane_on(addr, mobile::On::Again).await?;
            ui::activity(&format!("reaching this iPhone at {addr}"));
            dial(addr)
                .await
                .map_err(|e| mobile::not_mobile_data(unreached(addr, &e)))?
        }
        Err(e) => return Err(unreached(addr, &e)),
    };
    let mut rpc = RemotePairingClient::new(RpPairingSocket::new(stream), HOSTNAME);
    step("greeting the iPhone", 10, rpc.attempt_pair_verify()).await?;
    ui::activity("checking the pairing with it");
    match timeout(Duration::from_secs(15), rpc.validate_pairing(file)).await {
        Ok(Ok(())) => Ok(rpc),
        Ok(Err(e)) => Err(Fail::new(
            "This iPhone doesn't accept the pairing file (it was removed in Settings, or the \
             iPhone was reset). Run siboot again, and it pairs this iPhone again.",
            chain(&e),
        )),
        Err(_) => Err(Fail::new(
            "The iPhone stopped answering while checking the pairing.",
            "pair-verify timed out",
        )),
    }
}

/// A connection to the pairing service, in 10 s or a timeout error.
async fn dial(addr: SocketAddr) -> std::io::Result<TcpStream> {
    match timeout(Duration::from_secs(10), TcpStream::connect(addr)).await {
        Ok(dialed) => dialed,
        Err(_) => Err(std::io::Error::new(ErrorKind::TimedOut, "connect timed out")),
    }
}

/// Why `dial` didn't connect, as advice.
fn unreached(addr: SocketAddr, e: &std::io::Error) -> Fail {
    match e.kind() {
        ErrorKind::ConnectionRefused => Fail {
            refused: true,
            ..Fail::new(
                format!(
                    "This iPhone refused the connection at {addr}. On mobile data without Wi-Fi, \
                     iOS refuses it: run siboot again, and it offers Mobile Data Mode (beta). \
                     Otherwise turn on Developer Mode (Settings › Privacy & Security › Developer \
                     Mode), or restart the iPhone and connect LocalDevVPN again."
                ),
                e.to_string(),
            )
        },
        ErrorKind::TimedOut => Fail::new(
            format!("Nothing answered at {addr}. Connect LocalDevVPN (it shows Connected)."),
            e.to_string(),
        ),
        _ => Fail::new(
            format!("Couldn't reach this iPhone at {addr}. Connect LocalDevVPN (it shows Connected)."),
            e.to_string(),
        ),
    }
}

/// Await `future` for up to `secs`, showing `what` on the live screen while
/// it runs, and naming it if it fails or runs out.
pub async fn step<T, E: std::error::Error>(
    what: &str,
    secs: u64,
    future: impl std::future::Future<Output = std::result::Result<T, E>>,
) -> Result<T> {
    ui::activity(what);
    match timeout(Duration::from_secs(secs), future).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Fail::new(format!("Failed while {what}."), chain(&e))),
        Err(_) => Err(Fail::new(
            format!("Timed out while {what}."),
            format!("no answer within {secs} s"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dial_that_fails_says_what_to_do_in_plain_sentences() {
        let addr: SocketAddr = "10.7.0.1:49152".parse().unwrap();
        for kind in [
            ErrorKind::ConnectionRefused,
            ErrorKind::TimedOut,
            ErrorKind::PermissionDenied,
        ] {
            let f = unreached(addr, &std::io::Error::from(kind));
            assert!(!f.advice.contains("  "), "{}", f.advice);
            assert!(f.advice.contains("10.7.0.1:49152"), "{}", f.advice);
            // Only a refusal is what mobile data does (mobile.rs).
            assert_eq!(f.refused, kind == ErrorKind::ConnectionRefused, "{kind:?}");
        }
    }
}
