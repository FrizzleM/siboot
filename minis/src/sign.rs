//! Step 5: fetch SideStore and sign it for this iPhone, as SideInstaller signs
//! it (`Engine.performSign`): the device registered, then isideload's
//! `sign_app` with an app group and without Increased Memory Limit, which
//! SideStore, an app store that renews its own App ID, doesn't keep.
//!
//! isideload knows SideStore by its bundle ID and does for it what AltServer
//! does: the app group goes into its Info.plist and its widget's
//! (`ALTAppGroups`), and the signing certificate into the app
//! (`ALTCertificate.p12`, named by `ALTCertificateID`), so SideStore can sign
//! with the same certificate once you sign in to it.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use isideload::sideload::sideloader::MemoryLimit;

use crate::account::Session;
use crate::fail::{Fail, Result};
use crate::tunnel::DeviceInfo;
use crate::{ui, Args, VERSION};

/// SideStore's nightly, which SideInstaller's Nightly setting installs
/// (`InstallSource.downloadURL`). On iOS 27 it's the build that can use this
/// iPhone's pairing: an RPPairing record, which SideStore reads from its
/// nightly of 20 September 2026 on. Its newest release, 0.7.0-alpha, is a
/// build from 2024 that reads only a lockdown record, and iOS 27 makes none
/// and lets no app use one through LocalDevVPN.
pub const NIGHTLY_IPA: &str =
    "https://github.com/SideStore/SideStore/releases/download/nightly/SideStore.ipa";

/// SideStore's bundle ID as published, before signing adds the team ID:
/// the one isideload sets up as SideStore.
pub const SIDESTORE_BUNDLE_ID: &str = "com.SideStore.SideStore";

/// In a SideStore build that keeps one pairing file per protocol: the one it
/// loads an RPPairing record from.
const SPLIT_PAIRING_MARKER: &[u8] = b"PairingFile_RemoteRP";
/// Only in SideStore builds whose importer asks for a password before it takes
/// `Account.sideconf` (the `acctFileChecksum` setting). Those never take the
/// file ssboot would leave, and ask again on every launch, so they get none,
/// as SideInstaller checks (`Engine.importsAccountConfigSilently`). Version
/// numbers don't tell the two kinds apart.
const PROMPTING_IMPORTER_MARKER: &[u8] = b"acctFileChecksum";

/// An Info.plist is kilobytes; anything far bigger isn't one.
const MAX_INFO_PLIST: u64 = 4 << 20;

/// What the downloaded IPA is, read before signing it.
#[derive(Debug, PartialEq)]
pub struct Build {
    /// `CFBundleShortVersionString`, when it has one.
    pub version: Option<String>,
    /// It loads `PairingFile_RemoteRP.plist`, so it can use rootshell's pairing.
    pub reads_split_pairing: bool,
    /// It takes `Account.sideconf` without asking for a password.
    pub imports_account_silently: bool,
}

impl Build {
    /// The version, without the commit a nightly's carries
    /// ("0.7.0-20261027.1507+cd260851" is "0.7.0-20261027.1507").
    pub fn short_version(&self) -> Option<&str> {
        self.version.as_deref().map(|v| v.split('+').next().unwrap_or(v))
    }
}

pub struct Signed {
    /// The signed `.app` folder, ready to upload.
    pub app_dir: PathBuf,
    pub bundle_id: String,
    pub name: String,
}

/// `--ipa` as a path, or the URL it names, or SideStore's nightly.
pub async fn fetch_ipa(args: &Args) -> Result<PathBuf> {
    match args.ipa.as_deref() {
        Some(source) if !source.starts_with("https://") && !source.starts_with("http://") => {
            let path = PathBuf::from(source);
            check_ipa(&path)?;
            ui::ok(&format!("using {}", path.display()));
            Ok(path)
        }
        source => {
            let url = source.unwrap_or(NIGHTLY_IPA);
            let dir = args.home.join("downloads");
            std::fs::create_dir_all(&dir).map_err(|e| {
                Fail::new(format!("Couldn't create {}.", dir.display()), e.to_string())
            })?;
            download(url, &dir.join("SideStore.ipa")).await
        }
    }
}

async fn download(url: &str, dest: &Path) -> Result<PathBuf> {
    ui::status("downloading SideStore");
    ui::detail(url);
    let client = reqwest::Client::builder()
        .user_agent(format!("ssboot/{VERSION}"))
        .build()
        .map_err(|e| Fail::new("Couldn't start a download.", e.to_string()))?;
    let mut response = client.get(url).send().await.map_err(|e| {
        Fail::new(
            "Couldn't download SideStore. Check the internet connection (LocalDevVPN only \
             routes 10.7.0.1, so it isn't the cause).",
            crate::error_text::error_text(&e),
        )
    })?;
    if !response.status().is_success() {
        return Err(Fail::new(
            format!(
                "GitHub answered {} for the SideStore download. Try again in a minute.",
                response.status()
            ),
            url.to_string(),
        ));
    }
    let total = response.content_length();
    let partial = dest.with_extension("ipa.part");
    let mut file = std::fs::File::create(&partial).map_err(|e| {
        Fail::new(
            format!("Couldn't write {}.", partial.display()),
            e.to_string(),
        )
    })?;
    let mut received: u64 = 0;
    loop {
        let chunk = response.chunk().await.map_err(|e| {
            Fail::new(
                "The SideStore download broke off. Run ssboot again.",
                e.to_string(),
            )
        })?;
        let Some(chunk) = chunk else { break };
        file.write_all(&chunk).map_err(|e| {
            Fail::new(
                format!("Couldn't write {}.", partial.display()),
                e.to_string(),
            )
        })?;
        received += chunk.len() as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            ui::progress("downloaded", received, total);
        }
    }
    ui::progress_end();
    drop(file);
    std::fs::rename(&partial, dest)
        .map_err(|e| Fail::new(format!("Couldn't write {}.", dest.display()), e.to_string()))?;
    check_ipa(dest)?;
    ui::ok(&format!(
        "downloaded SideStore ({:.1} MB)",
        received as f64 / 1_048_576.0
    ));
    Ok(dest.to_path_buf())
}

/// An IPA is a zip: an error page or a truncated download isn't, and would
/// only fail later, inside the signer.
fn check_ipa(path: &Path) -> Result<()> {
    let mut magic = [0u8; 4];
    let read =
        std::fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic));
    match read {
        Ok(()) if &magic == b"PK\x03\x04" => Ok(()),
        Ok(()) => Err(Fail::new(
            format!(
                "{} isn't an IPA. GitHub may have returned an error page: try again in a minute.",
                path.display()
            ),
            format!("first bytes {magic:02x?}"),
        )),
        Err(e) => Err(Fail::new(
            format!("Couldn't read {}.", path.display()),
            e.to_string(),
        )),
    }
}

/// Reads the IPA's app before it's signed: that it's SideStore, since signing
/// spends one of the week's App IDs, and what this build reads.
pub fn inspect(ipa: &Path) -> Result<Build> {
    ui::activity("checking it's SideStore");
    let unreadable = |e: &dyn std::fmt::Display| {
        Fail::new(
            format!("Couldn't read {} as an IPA. Download it again.", ipa.display()),
            e.to_string(),
        )
    };
    let file = std::fs::File::open(ipa).map_err(|e| unreadable(&e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| unreadable(&e))?;
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    let info_name = main_info_plist(&names).ok_or_else(|| {
        Fail::new(
            format!("{} has no app in it.", ipa.display()),
            "no Payload/<name>.app/Info.plist",
        )
    })?;
    let mut info = Vec::new();
    archive
        .by_name(&info_name)
        .map_err(|e| unreadable(&e))?
        .take(MAX_INFO_PLIST)
        .read_to_end(&mut info)
        .map_err(|e| unreadable(&e))?;
    let info: plist::Dictionary = plist::from_bytes(&info).map_err(|e| unreadable(&e))?;
    let text = |key: &str| {
        info.get(key)
            .and_then(|v| v.as_string())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let bundle_id = text("CFBundleIdentifier").unwrap_or_default();
    if bundle_id != SIDESTORE_BUNDLE_ID {
        return Err(Fail::new(
            format!(
                "{} isn't SideStore: its app is {}. ssboot installs SideStore only \
                 ({SIDESTORE_BUNDLE_ID}), as published.",
                ipa.display(),
                if bundle_id.is_empty() { "unnamed" } else { bundle_id.as_str() },
            ),
            format!("CFBundleIdentifier {bundle_id:?}"),
        ));
    }
    let executable = text("CFBundleExecutable").ok_or_else(|| {
        Fail::new(
            format!("{} has no executable named in its Info.plist.", ipa.display()),
            "CFBundleExecutable missing",
        )
    })?;
    let app = info_name.trim_end_matches("Info.plist");
    let mut binary = Vec::new();
    archive
        .by_name(&format!("{app}{executable}"))
        .map_err(|e| unreadable(&e))?
        .read_to_end(&mut binary)
        .map_err(|e| unreadable(&e))?;
    Ok(Build {
        version: text("CFBundleShortVersionString"),
        reads_split_pairing: contains(&binary, SPLIT_PAIRING_MARKER),
        imports_account_silently: !contains(&binary, PROMPTING_IMPORTER_MARKER),
    })
}

/// The one app's own Info.plist: exactly `Payload/<name>.app/Info.plist`, not
/// an extension's or a framework's.
fn main_info_plist(names: &[String]) -> Option<String> {
    names
        .iter()
        .find(|name| {
            let parts: Vec<&str> = name.split('/').collect();
            parts.len() == 3
                && parts[0] == "Payload"
                && parts[1].ends_with(".app")
                && parts[2] == "Info.plist"
        })
        .cloned()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(haystack, needle).is_some()
}

pub async fn sign(session: &mut Session, ipa: &Path, device: &DeviceInfo) -> Result<Signed> {
    ui::status("signing; this takes a while here");
    // SideInstaller asks for Increased Memory Limit where it's safe, which
    // isideload turns off for SideStore: SideStore registers its App ID again
    // each week without it, so its next profile wouldn't grant it.
    let (app_dir, _special) = session
        .sideloader
        .sign_app(
            ipa.to_path_buf(),
            None,
            MemoryLimit::Off,
            true,
            Some((device.name.as_str(), device.udid.as_str())),
            None,
        )
        .await
        .map_err(|e| {
            let text = crate::error_text::report_text(&e);
            Fail::new(sign_advice(&text), text)
        })?;
    let (bundle_id, name) = bundle_info(&app_dir)?;
    ui::ok(&format!("signed {name} as {bundle_id}"));
    Ok(Signed {
        app_dir,
        bundle_id,
        name,
    })
}

/// Advice for the refusals SideInstaller's own signing explains.
fn sign_advice(text: &str) -> String {
    if text.starts_with("device registration failed") {
        "Apple wouldn't register this iPhone with your developer team, so it won't issue a \
         provisioning profile."
            .into()
    } else if text.contains("7460") {
        "Apple won't issue a signing certificate: it says one already exists (error 7460).".into()
    } else if text.contains("9401")
        || text
            .to_ascii_lowercase()
            .contains("maximum number of app ids")
    {
        "This Apple Account has reached its limit of App IDs for the week. Wait for one to expire, \
         or use another account."
            .into()
    } else {
        "Signing failed.".into()
    }
}

/// Bundle id and Home Screen name of the signed app.
fn bundle_info(app_dir: &Path) -> Result<(String, String)> {
    let info: plist::Dictionary = plist::from_file(app_dir.join("Info.plist"))
        .map_err(|e| Fail::new("The signed app has no readable Info.plist.", e.to_string()))?;
    let text = |key: &str| {
        info.get(key)
            .and_then(|v| v.as_string())
            .map(str::to_string)
    };
    let bundle_id = text("CFBundleIdentifier").ok_or_else(|| {
        Fail::new(
            "The signed app has no bundle identifier.",
            "CFBundleIdentifier missing",
        )
    })?;
    let name = text("CFBundleDisplayName")
        .filter(|n| !n.is_empty())
        .or_else(|| text("CFBundleName"))
        .unwrap_or_else(|| "SideStore".into());
    Ok((bundle_id, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ssboot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An IPA with `bundle_id`'s Info.plist and an executable holding `binary`,
    /// plus an extension, whose Info.plist mustn't be taken for the app's.
    fn an_ipa(dir: &Path, bundle_id: &str, binary: &[u8]) -> PathBuf {
        let path = dir.join(format!("{bundle_id}.ipa"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let plist = |id: &str, exec: &str| {
            let mut info = plist::Dictionary::new();
            info.insert("CFBundleIdentifier".into(), id.into());
            info.insert("CFBundleExecutable".into(), exec.into());
            info.insert(
                "CFBundleShortVersionString".into(),
                "0.7.0-20261027.1507+cd260851".into(),
            );
            let mut bytes = Vec::new();
            plist::to_writer_binary(&mut bytes, &info).unwrap();
            bytes
        };
        zip.start_file("Payload/SideStore.app/PlugIns/AltWidgetExtension.appex/Info.plist", options)
            .unwrap();
        zip.write_all(&plist("com.SideStore.SideStore.AltWidget", "AltWidgetExtension"))
            .unwrap();
        zip.start_file("Payload/SideStore.app/Info.plist", options).unwrap();
        zip.write_all(&plist(bundle_id, "SideStore")).unwrap();
        zip.start_file("Payload/SideStore.app/SideStore", options).unwrap();
        zip.write_all(binary).unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn only_a_zip_passes_as_an_ipa() {
        let dir = temp_dir("ipa");
        let good = dir.join("good.ipa");
        std::fs::write(&good, b"PK\x03\x04rest").unwrap();
        assert!(check_ipa(&good).is_ok());
        let page = dir.join("page.ipa");
        std::fs::write(&page, b"<!DOCTYPE html>").unwrap();
        assert!(check_ipa(&page)
            .unwrap_err()
            .advice
            .contains("isn't an IPA"));
        assert!(check_ipa(&dir.join("missing.ipa")).is_err());
    }

    #[test]
    fn a_nightly_reads_split_pairing_and_asks_before_importing() {
        let dir = temp_dir("nightly");
        let binary = b"..PairingFile_Lockdown.plist..PairingFile_RemoteRP.plist..acctFileChecksum..";
        let build = inspect(&an_ipa(&dir, SIDESTORE_BUNDLE_ID, binary)).unwrap();
        assert!(build.reads_split_pairing);
        assert!(!build.imports_account_silently);
        assert_eq!(build.short_version(), Some("0.7.0-20261027.1507"));
    }

    #[test]
    fn an_older_build_reads_one_file_and_imports_quietly() {
        let dir = temp_dir("older");
        let build = inspect(&an_ipa(&dir, SIDESTORE_BUNDLE_ID, b"..ALTPairingFile..")).unwrap();
        assert!(!build.reads_split_pairing);
        assert!(build.imports_account_silently);
    }

    #[test]
    fn anything_but_sidestore_is_refused_before_signing() {
        let dir = temp_dir("other");
        for id in ["com.kdt.livecontainer", "com.SideStore.SideStore.ABCDE12345"] {
            let failure = inspect(&an_ipa(&dir, id, b"")).unwrap_err();
            assert!(failure.advice.contains("isn't SideStore"), "{}", failure.advice);
            assert!(failure.advice.contains(id), "{}", failure.advice);
        }
    }

    #[test]
    fn the_apps_own_info_plist_is_found_among_its_extensions() {
        let names: Vec<String> = [
            "Payload/",
            "Payload/SideStore.app/PlugIns/AltWidgetExtension.appex/Info.plist",
            "Payload/SideStore.app/Frameworks/OpenSSL.framework/Info.plist",
            "Payload/SideStore.app/Info.plist",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            main_info_plist(&names).as_deref(),
            Some("Payload/SideStore.app/Info.plist")
        );
        assert_eq!(main_info_plist(&names[..3]), None);
    }

    /// Step 5's read of a real SideStore IPA, named by `SSBOOT_SIDESTORE_IPA`:
    /// `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_real_sidestore_ipa_reads_as_sidestore() {
        let path = std::env::var_os("SSBOOT_SIDESTORE_IPA")
            .expect("SSBOOT_SIDESTORE_IPA names a SideStore IPA");
        let build = inspect(Path::new(&path)).unwrap();
        println!("{build:?}");
        assert!(build.version.is_some());
    }

    #[test]
    fn the_bundle_id_and_name_come_from_info_plist() {
        let dir = temp_dir("app").join("SideStore.app");
        std::fs::create_dir_all(&dir).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert(
            "CFBundleIdentifier".into(),
            "com.SideStore.SideStore.ABCDE12345".into(),
        );
        info.insert("CFBundleName".into(), "SideStore".into());
        plist::to_file_xml(dir.join("Info.plist"), &info).unwrap();
        let (id, name) = bundle_info(&dir).unwrap();
        assert_eq!(id, "com.SideStore.SideStore.ABCDE12345");
        assert_eq!(name, "SideStore");
    }
}
