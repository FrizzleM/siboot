//! Step 7: hand the newly installed AltStore this iPhone's pairing, where
//! AltServer puts it after installing AltStore, as SideInstaller does since
//! 2.0 (`Engine.handOffAltStorePairing` and `AltStorePairingHandoff` in its
//! ios-app): the RPPairing record on its own, as an XML plist, at
//! `Documents/ALTPairingFile.plist` in AltStore's container
//! (altstoreio/AltStore 5811912, 23 September 2026, in 2.3.1 and later).
//! AltStore takes it into its Keychain when its Remote AltServer is set up,
//! which then skips pairing.
//!
//! Only an RPPairing record connects: AltStore's `OnDeviceClient` refuses one
//! without `private_key`. A lockdown record would be taken all the same and
//! then fail, so none is written without an RPPairing one, and AltStore pairs
//! during that setup instead. The pairing made in Minis or by altboot-pair is
//! always one. AltStore 2.3 read the record from its own bundle instead, where
//! step 5 has isideload seal it.
//!
//! It's optional, as in SideInstaller: AltStore works without the file, so
//! failing to write it is a warning, and the run still ends installed.
//!
//! house_arrest vends the whole container because the app is signed for
//! development.

use std::path::Path;

use idevice::afc::opcode::AfcFopenMode;
use idevice::afc::AfcClient;
use idevice::house_arrest::HouseArrestClient;
use idevice::remote_pairing::RpPairingFile;
use idevice::RsdService;
use tracing::subscriber::NoSubscriber;

use crate::fail::{chain, Fail, Result};
use crate::tunnel::Connection;
use crate::ui;

/// Where AltStore reads the record, in its Documents.
pub const PAIRING_FILE: &str = "ALTPairingFile.plist";

/// The RPPairing record in the pairing file at `path`, as idevice writes it
/// and AltServer hands it over (`rp_pairing_file_to_bytes`): `identifier`,
/// `public_key`, `private_key` and `alt_irk`, as XML. None when the file has
/// none AltStore could connect with (rust-core's `altstore_pairing_file`).
pub fn altstore_record(path: &Path) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    record_from(&data)
}

fn record_from(data: &[u8]) -> Option<Vec<u8>> {
    // idevice logs the record it parses, private key included, at debug level.
    let parsed = tracing::subscriber::with_default(NoSubscriber::default(), || {
        RpPairingFile::from_bytes(data)
    });
    parsed.ok().map(|record| record.to_bytes())
}

/// Step 7, for AltStore signed as `bundle_id`. `record` is from
/// [`altstore_record`]. True once AltStore has the file.
pub async fn hand_over(connection: &mut Connection, bundle_id: &str, record: Option<&[u8]>) -> bool {
    let Some(record) = record else {
        ui::note(
            "This pairing file has no RPPairing record, the only kind AltStore connects with, so \
             none was written. Setting up AltStore's Remote AltServer pairs it instead.",
        );
        return false;
    };
    match write(connection, bundle_id, record).await {
        Ok(written) => {
            ui::ok(&format!(
                "pairing file written into AltStore ({written} bytes, read back and checked)"
            ));
            ui::detail(&format!("/Documents/{PAIRING_FILE} in {bundle_id}"));
            true
        }
        Err(f) => {
            ui::warn(
                "Couldn't put the pairing file into AltStore. It's installed: setting up its \
                 Remote AltServer pairs it instead.",
            );
            ui::detail(&f.advice);
            ui::detail(&f.raw);
            false
        }
    }
}

async fn write(connection: &mut Connection, bundle_id: &str, record: &[u8]) -> Result<usize> {
    ui::activity("opening AltStore's container");
    let house_arrest =
        HouseArrestClient::connect_rsd(&mut connection.adapter, &mut connection.handshake)
            .await
            .map_err(|e| Fail::new("Couldn't reach house_arrest on this iPhone.", chain(&e)))?;
    let mut afc = house_arrest
        .vend_container(bundle_id)
        .await
        .map_err(|e| Fail::new(format!("Couldn't open {bundle_id}'s container."), chain(&e)))?;
    ui::activity("writing the pairing file into it");
    let _ = afc.mk_dir("/Documents").await;
    write_verified(&mut afc, &format!("/Documents/{PAIRING_FILE}"), record).await
}

async fn read_if_present(afc: &mut AfcClient, path: &str) -> Result<Option<Vec<u8>>> {
    if afc.get_file_info(path).await.is_err() {
        return Ok(None);
    }
    let mut file = afc
        .open(path, AfcFopenMode::RdOnly)
        .await
        .map_err(|e| Fail::new(format!("Couldn't open {path}."), chain(&e)))?;
    let bytes = file
        .read_entire()
        .await
        .map_err(|e| Fail::new(format!("Couldn't read {path}."), chain(&e)))?;
    let _ = file.close().await;
    Ok(Some(bytes))
}

/// Write `data` to `path`, then read it back and compare. Returns its length.
async fn write_verified(afc: &mut AfcClient, path: &str, data: &[u8]) -> Result<usize> {
    let mut file = afc
        .open(path, AfcFopenMode::WrOnly)
        .await
        .map_err(|e| Fail::new(format!("Couldn't create {path}."), chain(&e)))?;
    file.write_entire(data)
        .await
        .map_err(|e| Fail::new(format!("Couldn't write {path}."), chain(&e)))?;
    file.close()
        .await
        .map_err(|e| Fail::new(format!("Couldn't write {path}."), chain(&e)))?;
    match read_if_present(afc, path).await? {
        Some(back) if back == data => Ok(data.len()),
        Some(back) => Err(Fail::new(
            format!("{path} reads back different from what was written."),
            format!("wrote {} bytes, read {}", data.len(), back.len()),
        )),
        None => Err(Fail::new(format!("{path} vanished after writing."), "")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::{Dictionary, Value};

    fn keys(xml: &[u8]) -> Vec<String> {
        let dict: Dictionary = plist::from_bytes(xml).unwrap();
        let mut keys: Vec<String> = dict.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// What pairing in Minis or altboot-pair saves: an RPPairing record, as
    /// idevice writes it.
    fn a_pairing() -> RpPairingFile {
        let mut file = RpPairingFile::generate("altboot test");
        file.alt_irk = Some(vec![7; 16]);
        file
    }

    #[test]
    fn altstore_gets_the_rppairing_record_as_xml() {
        let pairing = a_pairing();
        let record = record_from(&pairing.to_bytes()).unwrap();
        assert!(record.starts_with(b"<?xml"), "AltServer writes XML");
        assert_eq!(keys(&record), ["alt_irk", "identifier", "private_key", "public_key"]);
        let back = RpPairingFile::from_bytes(&record).unwrap();
        assert_eq!(back.identifier, pairing.identifier);
        assert_eq!(back.public_key_bytes(), pairing.public_key_bytes());
    }

    #[test]
    fn a_merged_file_gives_only_its_rppairing_record() {
        let mut merged: Dictionary = plist::from_bytes(&a_pairing().to_bytes()).unwrap();
        for key in ["HostID", "SystemBUID", "HostCertificate", "UDID"] {
            merged.insert(key.into(), Value::String(key.into()));
        }
        let mut bytes = Vec::new();
        plist::to_writer_binary(&mut bytes, &merged).unwrap();
        let record = record_from(&bytes).unwrap();
        assert_eq!(keys(&record), ["alt_irk", "identifier", "private_key", "public_key"]);
    }

    #[test]
    fn a_file_without_an_rppairing_record_gives_none() {
        let mut lockdown = Dictionary::new();
        lockdown.insert("HostID".into(), Value::String("x".into()));
        lockdown.insert("public_key".into(), Value::Data(vec![1; 32]));
        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, &lockdown).unwrap();
        assert_eq!(record_from(&bytes), None, "no private_key: AltStore refuses it");
        assert_eq!(record_from(b"not a plist"), None);
        assert_eq!(altstore_record(Path::new("/nonexistent/rp_pairing_file.plist")), None);
    }
}
