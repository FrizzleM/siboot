//! Step 7: hand the newly installed SideStore this iPhone's pairing, and the
//! settings it signs in with, as SideInstaller does once it has installed
//! SideStore (`Engine.performWritePairing`, `handOffSplitPairing` and
//! `handOffSideStoreAnisette` in ../SideInstaller/ios-app/Engine.swift; the
//! formats are `SideStorePairingHandoff` and `SideStoreAnisetteHandoff` in
//! PairingTargets.swift).
//!
//! SideStore keeps its pairing in its Documents. Its builds before the nightly
//! of 20 September 2026 read `ALTPairingFile.mobiledevicepairing`. The
//! nightlies since keep one file per protocol (`PairingFile_Lockdown.plist`,
//! `PairingFile_RemoteRP.plist`), turn down a file holding both, and load
//! neither until their own import has turned `isPairingReset` off and noted
//! the protocol in `activePairingProtocol`. All of that is written, so either
//! kind of build connects from its first launch without asking for a pairing
//! file. On iOS 27 the pairing from rootshell is an RPPairing record alone:
//! iOS 27 makes no lockdown record, and lets no app use one through
//! LocalDevVPN.
//!
//! Then SideStore's anisette settings, set the way its users got past Apple's
//! -80046, and, for a SideStore build that takes it without asking for a
//! password, the signing certificate (`Account.sideconf`), so its first
//! sign-in reuses it.
//!
//! house_arrest vends the whole container because the app is signed for
//! development. The settings go in after the install: installing is what makes
//! iOS read SideStore's settings file again.

use std::path::Path;

use idevice::afc::opcode::AfcFopenMode;
use idevice::afc::AfcClient;
use idevice::house_arrest::HouseArrestClient;
use idevice::RsdService;
use plist::{Dictionary, Value};

use crate::fail::{chain, Fail, Result};
use crate::tunnel::{Connection, DeviceInfo};
use crate::ui;

/// What SideStore read before its nightly of 20 September 2026, and still
/// reads in LiveContainer: either record, or both, in one file.
pub const LEGACY_FILE: &str = "ALTPairingFile.mobiledevicepairing";
/// The nightlies' file for each protocol.
pub const LOCKDOWN_FILE: &str = "PairingFile_Lockdown.plist";
pub const REMOTE_FILE: &str = "PairingFile_RemoteRP.plist";
/// The certificate hand-off, which SideStore imports and deletes on launch.
pub const ACCOUNT_FILE: &str = "Account.sideconf";

/// The keys minimuxer requires for each protocol (MinimuxerCommon's
/// `PairingFile.swift`).
const REQUIRED_LOCKDOWN_KEYS: [&str; 10] = [
    "WiFiMACAddress",
    "SystemBUID",
    "RootPrivateKey",
    "HostPrivateKey",
    "HostID",
    "RootCertificate",
    "UDID",
    "EscrowBag",
    "HostCertificate",
    "DeviceCertificate",
];
const REQUIRED_REMOTE_KEYS: [&str; 3] = ["private_key", "public_key", "identifier"];
/// Every RPPairing key, the optional `alt_irk` too.
const REMOTE_KEYS: [&str; 4] = ["private_key", "public_key", "identifier", "alt_irk"];

/// "SideStore (New)" on SideStore's list of anisette servers
/// (servers.sidestore.io), spelled as the list has it: SideStore starts on the
/// first server when the one picked isn't on it.
pub const ANISETTE_SERVER: &str = "https://new.ani.sidestore.io";
/// The anisette servers on Apple's macOS ADI library, which Apple still issues
/// sign-in tokens through (SideInstaller's `AnisetteServer.macADIOrder`).
pub const MAC_ADI_HOSTS: [&str; 6] = [
    "new.ani.sidestore.io",
    "mac-ani.jkcoxson.com",
    "ani.iloader.app",
    "ani3.stikstore.app",
    "ani.jkcoxson.com",
    "ani2.stikstore.app",
];
/// SideStore's default headers (AnisetteKit's `defaultClientInfo` and
/// `defaultUserAgent`): a current Mac.
const CLIENT_INFO: &str =
    "<MacBookPro18,3> <macOS;26.6;25F84> <com.apple.AuthKit/1 (com.apple.akd/1.0)>";
const USER_AGENT: &str = "AuthKit/1 (Macintosh; OS X 26.6) (com.apple.akd/1.0)";

/// What went into SideStore, for the screen.
pub struct Handed {
    /// Whether `Account.sideconf` went in.
    pub certificate: bool,
}

/// The records split out of a pairing file, each an XML plist: SideStore
/// reads its pairing files as UTF-8 strings.
#[derive(Debug)]
pub struct Records {
    /// The lockdown record alone, or None if the file has no complete one.
    pub lockdown: Option<Vec<u8>>,
    /// The RPPairing record alone, or None if the file has none.
    pub remote: Option<Vec<u8>>,
    /// Keys missing from a partial lockdown record, for the log.
    pub missing_lockdown_keys: Vec<&'static str>,
}

impl Records {
    /// The protocol SideStore should load. Where lockdown works over the
    /// loopback VPN it goes first, as in SideInstaller: SideStore reaches
    /// lockdownd that way, while the RPPairing listener can refuse a tunnel
    /// from the device itself. From iOS 26.4 on lockdownd hangs up on it
    /// there, so RPPairing goes first. A file from rootshell has only an
    /// RPPairing record anyway.
    pub fn active_protocol(&self, lockdown_works: bool) -> Option<&'static str> {
        match (&self.lockdown, &self.remote) {
            (Some(_), _) if lockdown_works => Some("lockdown"),
            (_, Some(_)) => Some("rppairing"),
            (Some(_), None) => Some("lockdown"),
            (None, None) => None,
        }
    }
}

/// False from iOS 26.4 on, where lockdownd hangs up on a connection that
/// reaches it through a loopback VPN (SideInstaller's
/// `Engine.lockdownWorksOverLoopback`), and for a version that can't be read:
/// ssboot runs on iOS 27.
pub fn lockdown_works_over_loopback(ios_version: &str) -> bool {
    let mut parts = ios_version.split('.').map(|p| p.trim().parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten().unwrap_or(0)) {
        (Some(major), minor) => (major, minor) < (26, 4),
        (None, _) => false,
    }
}

/// Splits a pairing file (lockdown, RPPairing, or both merged) into one record
/// per protocol, adding the UDID the lockdown record needs if it's missing.
pub fn split(pairing: &Dictionary, udid: Option<&str>) -> Result<Records> {
    let mut lockdown: Dictionary = pairing
        .iter()
        .filter(|(key, _)| !REMOTE_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    stamp_udid(&mut lockdown, udid);
    let missing: Vec<&'static str> = REQUIRED_LOCKDOWN_KEYS
        .iter()
        .copied()
        .filter(|key| !lockdown.contains_key(key))
        .collect();
    // A file with no lockdown keys at all isn't missing anything worth saying.
    let has_any_lockdown = REQUIRED_LOCKDOWN_KEYS
        .iter()
        .any(|key| *key != "UDID" && lockdown.contains_key(key));

    let remote: Dictionary = pairing
        .iter()
        .filter(|(key, _)| REMOTE_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let has_remote = REQUIRED_REMOTE_KEYS.iter().all(|key| remote.contains_key(key));

    Ok(Records {
        lockdown: if missing.is_empty() { Some(xml(&lockdown)?) } else { None },
        remote: if has_remote { Some(xml(&remote)?) } else { None },
        missing_lockdown_keys: if has_any_lockdown { missing } else { Vec::new() },
    })
}

/// `ALTPairingFile.mobiledevicepairing`: the whole pairing file, as XML, with
/// the UDID stamped into a lockdown record that lacks one (pymobiledevice3
/// and idevicepair leave it out, and minimuxer needs it).
pub fn legacy_file(pairing: &Dictionary, udid: Option<&str>) -> Result<Vec<u8>> {
    let mut file = pairing.clone();
    let has_lockdown = REQUIRED_LOCKDOWN_KEYS
        .iter()
        .any(|key| *key != "UDID" && file.contains_key(key));
    if has_lockdown {
        stamp_udid(&mut file, udid);
    }
    xml(&file)
}

fn stamp_udid(record: &mut Dictionary, udid: Option<&str>) {
    let Some(udid) = udid.filter(|u| !u.is_empty()) else {
        return;
    };
    let has_one = record
        .get("UDID")
        .and_then(Value::as_string)
        .is_some_and(|u| !u.is_empty());
    if !has_one {
        record.insert("UDID".into(), udid.into());
    }
}

/// What SideStore's own import sets: pairing no longer reset, and the
/// protocol whose file to load. A protocol picked in SideStore's settings
/// lives in `preferredPairingProtocol`, which is left alone.
pub fn apply_pairing_settings(prefs: &mut Dictionary, protocol: &str) {
    prefs.insert("isPairingReset".into(), false.into());
    prefs.insert("activePairingProtocol".into(), protocol.into());
}

/// Sets SideStore to sign in the way its users got past Apple's -80046
/// (SideStore #1685, #1686): on-device anisette and auto-rotation off, and a
/// macOS-ADI server, keeping one already picked. Client info saved for a
/// Windows PC or an Android phone, as guides for -80045 had people do, goes
/// back to SideStore's default Mac. Returns what changed, for the screen.
///
/// The reset of adi.pb those users also did can't be done from here: it's in
/// SideStore's keychain.
pub fn apply_anisette_settings(prefs: &mut Dictionary) -> Vec<&'static str> {
    let mut changes = Vec::new();
    if prefs.get("useOnDeviceAnisette").and_then(Value::as_boolean) != Some(false) {
        prefs.insert("useOnDeviceAnisette".into(), false.into());
        changes.push("on-device anisette off");
    }
    if prefs.get("disableAnisetteRotation").and_then(Value::as_boolean) != Some(true) {
        prefs.insert("disableAnisetteRotation".into(), true.into());
        changes.push("auto-rotation off");
    }
    let picked = prefs
        .get("menuAnisetteURL")
        .and_then(Value::as_string)
        .unwrap_or("");
    if !MAC_ADI_HOSTS.contains(&host_of(picked).to_ascii_lowercase().as_str()) {
        prefs.insert("menuAnisetteURL".into(), ANISETTE_SERVER.into());
        changes.push("server SideStore (New)");
    }
    let saved = prefs
        .get("customAnisetteClientInfo")
        .and_then(Value::as_string)
        .unwrap_or("");
    if !saved.is_empty() && !saved.contains("<macOS;") && !saved.contains("<Mac OS X;") {
        prefs.insert("customAnisetteClientInfo".into(), CLIENT_INFO.into());
        prefs.insert("customAnisetteUserAgent".into(), USER_AGENT.into());
        changes.push("client info back to a Mac");
    }
    changes
}

/// The host of a URL as SideStore saves it ("https://host/..."), or "".
fn host_of(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else {
        return "";
    };
    rest.split(['/', ':', '?', '#']).next().unwrap_or("")
}

fn xml(dict: &Dictionary) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    plist::to_writer_xml(&mut out, dict)
        .map_err(|e| Fail::new("Couldn't encode the pairing for SideStore.", e.to_string()))?;
    Ok(out)
}

/// The pairing file at `path`, as a dictionary.
fn read_pairing(path: &Path) -> Result<Dictionary> {
    let bytes = std::fs::read(path)
        .map_err(|e| Fail::new(format!("Couldn't read {}.", path.display()), e.to_string()))?;
    plist::from_bytes::<Dictionary>(&bytes).map_err(|e| {
        Fail::new(
            format!("{} isn't a pairing file.", path.display()),
            e.to_string(),
        )
    })
}

/// Step 7, for the iPhone `device`. `account_config` is `Account.sideconf`,
/// for a build that takes it without asking.
pub async fn hand_over(
    connection: &mut Connection,
    bundle_id: &str,
    pairing_path: &Path,
    device: &DeviceInfo,
    account_config: Option<&str>,
) -> Result<Handed> {
    let udid = device.udid.as_str();
    let pairing = read_pairing(pairing_path)?;
    let legacy = legacy_file(&pairing, Some(udid))?;
    let records = split(&pairing, Some(udid))?;
    if !records.missing_lockdown_keys.is_empty() {
        ui::warn(&format!(
            "The pairing's lockdown record lacks {}, which newer SideStore builds need, so it's \
             left out of their files.",
            records.missing_lockdown_keys.join(", ")
        ));
    }
    let lockdown_works = lockdown_works_over_loopback(&device.ios_version);
    let Some(protocol) = records.active_protocol(lockdown_works) else {
        return Err(Fail::new(
            "The pairing file has no record SideStore can load.",
            format!("{} has neither an RPPairing nor a lockdown record", pairing_path.display()),
        ));
    };

    ui::activity("opening SideStore's container");
    let house_arrest =
        HouseArrestClient::connect_rsd(&mut connection.adapter, &mut connection.handshake)
            .await
            .map_err(|e| Fail::new("Couldn't reach house_arrest on this iPhone.", chain(&e)))?;
    let mut afc = house_arrest.vend_container(bundle_id).await.map_err(|e| {
        Fail::new(
            format!(
                "SideStore is installed, but ssboot couldn't open its container ({bundle_id}) to \
                 give it the pairing. Run ssboot again."
            ),
            chain(&e),
        )
    })?;

    ui::activity("writing the pairing files into it");
    let _ = afc.mk_dir("/Documents").await;
    let mut files = vec![(LEGACY_FILE, legacy)];
    if let Some(remote) = records.remote {
        files.push((REMOTE_FILE, remote));
    }
    if let Some(lockdown) = records.lockdown {
        files.push((LOCKDOWN_FILE, lockdown));
    }
    for (name, data) in &files {
        let written = write_verified(&mut afc, &format!("/Documents/{name}"), data).await?;
        ui::detail(&format!("/Documents/{name}: {written} bytes, read back and checked"));
    }
    ui::ok(&format!(
        "pairing files written into SideStore ({} of them, each read back)",
        files.len()
    ));

    ui::activity("setting SideStore to load them");
    let prefs_path = format!("/Library/Preferences/{bundle_id}.plist");
    let mut prefs = match read_if_present(&mut afc, &prefs_path).await? {
        Some(bytes) if !bytes.is_empty() => {
            plist::from_bytes::<Dictionary>(&bytes).map_err(|e| {
                Fail::new(
                    "SideStore's settings file isn't a plist, so ssboot left it alone. Delete \
                     SideStore and run ssboot again.",
                    e.to_string(),
                )
            })?
        }
        _ => Dictionary::new(),
    };
    apply_pairing_settings(&mut prefs, protocol);
    let anisette = apply_anisette_settings(&mut prefs);
    save_prefs(&mut afc, &prefs_path, &prefs).await?;
    ui::ok(&format!(
        "SideStore loads the {} from its first launch",
        if protocol == "rppairing" { "RPPairing file" } else { "lockdown pairing" }
    ));
    ui::detail(&format!("isPairingReset off, activePairingProtocol {protocol} ({prefs_path})"));
    if !anisette.is_empty() {
        ui::ok(&format!(
            "SideStore's anisette set as its users get past -80046: {}",
            anisette.join(", ")
        ));
    }

    let mut certificate = false;
    if let Some(config) = account_config {
        let path = format!("/Documents/{ACCOUNT_FILE}");
        match write_verified(&mut afc, &path, config.as_bytes()).await {
            Ok(written) => {
                certificate = true;
                ui::ok("signing certificate handed over: SideStore takes it in on first launch");
                ui::detail(&format!("{path}: {written} bytes, read back and checked"));
            }
            Err(f) => {
                ui::warn(
                    "Couldn't hand SideStore the signing certificate. It works all the same, but \
                     may offer to resign itself the first time you sign in.",
                );
                ui::detail(&f.raw);
            }
        }
    }
    Ok(Handed { certificate })
}

/// Writes SideStore's settings beside the file and renames them over it, as
/// cfprefsd saves them: a broken upload can't leave half a file, and an
/// in-place rewrite would keep a copy cfprefsd may have cached.
async fn save_prefs(afc: &mut AfcClient, path: &str, prefs: &Dictionary) -> Result<()> {
    let mut bytes = Vec::new();
    plist::to_writer_binary(&mut bytes, prefs)
        .map_err(|e| Fail::new("Couldn't encode SideStore's settings.", e.to_string()))?;
    let _ = afc.mk_dir("/Library").await;
    let _ = afc.mk_dir("/Library/Preferences").await;
    let staging = format!("{path}.ssboot");
    write_verified(afc, &staging, &bytes).await?;
    afc.rename(staging, path)
        .await
        .map_err(|e| Fail::new("Couldn't save SideStore's settings.", chain(&e)))?;
    Ok(())
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
    use idevice::remote_pairing::RpPairingFile;

    fn parse(bytes: &[u8]) -> Dictionary {
        plist::from_bytes(bytes).unwrap()
    }

    /// What rootshell hands over: an RPPairing record, as idevice writes it.
    fn from_rootshell() -> Dictionary {
        let mut file = RpPairingFile::generate("ssboot test");
        file.alt_irk = Some(vec![7; 16]);
        parse(&file.to_bytes())
    }

    fn a_lockdown_record() -> Dictionary {
        REQUIRED_LOCKDOWN_KEYS
            .iter()
            .filter(|key| **key != "UDID")
            .map(|key| (key.to_string(), Value::Data(key.as_bytes().to_vec())))
            .collect()
    }

    #[test]
    fn rootshells_pairing_goes_in_as_rppairing() {
        let pairing = from_rootshell();
        let records = split(&pairing, Some("00008150-0001")).unwrap();
        assert_eq!(records.active_protocol(false), Some("rppairing"));
        assert_eq!(records.active_protocol(true), Some("rppairing"));
        assert!(records.lockdown.is_none());
        assert!(records.missing_lockdown_keys.is_empty(), "nothing to say about no lockdown keys");
        let remote = parse(records.remote.as_deref().unwrap());
        let mut keys: Vec<&str> = remote.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["alt_irk", "identifier", "private_key", "public_key"]);
        // Still the record idevice (and so SideStore) reads.
        assert!(RpPairingFile::from_bytes(records.remote.as_deref().unwrap()).is_ok());
        // XML: SideStore reads the file as a string.
        assert!(records.remote.unwrap().starts_with(b"<?xml"));
    }

    #[test]
    fn the_legacy_file_is_the_whole_pairing_as_xml() {
        let pairing = from_rootshell();
        let legacy = legacy_file(&pairing, Some("00008150-0001")).unwrap();
        assert!(legacy.starts_with(b"<?xml"));
        assert_eq!(parse(&legacy), pairing, "no UDID is stamped into an RPPairing record");
    }

    #[test]
    fn a_merged_file_splits_in_two_and_loads_what_works_there() {
        let mut pairing = from_rootshell();
        for (key, value) in a_lockdown_record() {
            pairing.insert(key, value);
        }
        let records = split(&pairing, Some("00008150-0001")).unwrap();
        assert_eq!(records.active_protocol(true), Some("lockdown"), "before iOS 26.4");
        assert_eq!(records.active_protocol(false), Some("rppairing"), "iOS 27");
        let lockdown = parse(records.lockdown.as_deref().unwrap());
        assert_eq!(lockdown.get("UDID").and_then(Value::as_string), Some("00008150-0001"));
        assert!(!lockdown.contains_key("private_key"), "each file holds one record");
        let remote = parse(records.remote.as_deref().unwrap());
        assert!(!remote.contains_key("HostID"));
        let legacy = parse(&legacy_file(&pairing, Some("00008150-0001")).unwrap());
        assert_eq!(legacy.get("UDID").and_then(Value::as_string), Some("00008150-0001"));
        assert!(legacy.contains_key("private_key"), "the legacy file keeps both");
    }

    #[test]
    fn a_partial_lockdown_record_is_left_out_and_said() {
        let mut pairing = from_rootshell();
        pairing.insert("HostCertificate".into(), Value::Data(vec![1]));
        let records = split(&pairing, None).unwrap();
        assert!(records.lockdown.is_none());
        assert!(records.missing_lockdown_keys.contains(&"EscrowBag"));
        assert!(records.missing_lockdown_keys.contains(&"UDID"));
        assert_eq!(records.active_protocol(true), Some("rppairing"));
    }

    #[test]
    fn a_lockdown_record_alone_is_loaded_even_where_it_may_fail() {
        let records = split(&a_lockdown_record(), Some("00008150-0001")).unwrap();
        assert_eq!(records.active_protocol(false), Some("lockdown"));
    }

    #[test]
    fn a_file_with_neither_record_loads_nothing() {
        let mut junk = Dictionary::new();
        junk.insert("hello".into(), "there".into());
        assert_eq!(split(&junk, Some("x")).unwrap().active_protocol(true), None);
    }

    #[test]
    fn lockdown_works_over_the_loopback_only_before_ios_26_4() {
        for (version, works) in [
            ("26.3.1", true),
            ("18.6", true),
            ("26.4", false),
            ("26.6.2", false),
            ("27.1", false),
            ("27", false),
            ("", false),
            ("unknown", false),
        ] {
            assert_eq!(lockdown_works_over_loopback(version), works, "{version}");
        }
    }

    #[test]
    fn the_pairing_settings_are_what_sidestores_import_sets() {
        let mut prefs = Dictionary::new();
        prefs.insert("isPairingReset".into(), true.into());
        prefs.insert("preferredPairingProtocol".into(), "lockdown".into());
        apply_pairing_settings(&mut prefs, "rppairing");
        assert_eq!(prefs.get("isPairingReset").and_then(Value::as_boolean), Some(false));
        assert_eq!(
            prefs.get("activePairingProtocol").and_then(Value::as_string),
            Some("rppairing")
        );
        assert_eq!(
            prefs.get("preferredPairingProtocol").and_then(Value::as_string),
            Some("lockdown"),
            "the user's own pick is left alone"
        );
    }

    #[test]
    fn a_fresh_sidestore_gets_a_mac_adi_server() {
        let mut prefs = Dictionary::new();
        let changes = apply_anisette_settings(&mut prefs);
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert_eq!(prefs.get("useOnDeviceAnisette").and_then(Value::as_boolean), Some(false));
        assert_eq!(prefs.get("disableAnisetteRotation").and_then(Value::as_boolean), Some(true));
        assert_eq!(
            prefs.get("menuAnisetteURL").and_then(Value::as_string),
            Some(ANISETTE_SERVER)
        );
        assert!(!prefs.contains_key("customAnisetteClientInfo"));
        assert!(apply_anisette_settings(&mut prefs).is_empty(), "a second pass changes nothing");
    }

    #[test]
    fn a_mac_adi_pick_and_mac_client_info_are_kept() {
        let mut prefs = Dictionary::new();
        prefs.insert("menuAnisetteURL".into(), "https://ani.iloader.app/".into());
        prefs.insert("customAnisetteClientInfo".into(), CLIENT_INFO.into());
        apply_anisette_settings(&mut prefs);
        assert_eq!(
            prefs.get("menuAnisetteURL").and_then(Value::as_string),
            Some("https://ani.iloader.app/")
        );
        assert_eq!(
            prefs.get("customAnisetteClientInfo").and_then(Value::as_string),
            Some(CLIENT_INFO)
        );
    }

    #[test]
    fn android_server_and_windows_client_info_go_back_to_a_mac() {
        let mut prefs = Dictionary::new();
        prefs.insert("menuAnisetteURL".into(), "https://ani.sidestore.io".into());
        prefs.insert(
            "customAnisetteClientInfo".into(),
            "<PC> <Windows;6.2(0,0);9200> <com.apple.AuthKit/1 (com.apple.iCloud/7.21)>".into(),
        );
        let changes = apply_anisette_settings(&mut prefs);
        assert!(changes.contains(&"server SideStore (New)"));
        assert!(changes.contains(&"client info back to a Mac"));
        assert_eq!(
            prefs.get("customAnisetteUserAgent").and_then(Value::as_string),
            Some(USER_AGENT)
        );
    }

    #[test]
    fn hosts_come_out_of_urls() {
        assert_eq!(host_of("https://new.ani.sidestore.io"), "new.ani.sidestore.io");
        assert_eq!(host_of("https://ani.iloader.app:443/v3/"), "ani.iloader.app");
        assert_eq!(host_of("ani.iloader.app"), "");
        assert_eq!(host_of(""), "");
    }

    /// The server SideStore is set to, and every macOS-ADI host it may keep,
    /// are ones ssboot's own sign-in tries.
    #[test]
    fn the_mac_adi_hosts_are_ssboots_own_servers() {
        assert!(MAC_ADI_HOSTS.contains(&host_of(ANISETTE_SERVER)));
        for host in MAC_ADI_HOSTS {
            assert!(
                crate::account::SERVERS.iter().any(|s| host_of(s) == host),
                "{host}"
            );
        }
    }
}
