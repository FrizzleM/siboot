//! Step 5: fetch AltStore and sign it for this iPhone, as SideInstaller
//! installs AltStore from its Install tab (since 2.0: `InstallSource.altStore`
//! in its ios-app/SideStoreDownloader.swift, and the vendored isideload this
//! crate builds on):
//!
//! - The download comes from AltStore's own source, `apps.altstore.io`, the
//!   list AltServer installs AltStore from (`altstoreSourceURL` in AltServer's
//!   `ALTDeviceManager+Installation.swift`): the first version it lists, from
//!   2.3 on, that runs on this iPhone's iOS, as AltServer takes it. Only that
//!   file goes on: it has to match the SHA-256 the source lists for it.
//! - The IPA is read before signing, since signing spends one of the week's
//!   App IDs: anything but AltStore Classic (`com.rileytestut.AltStore`) is
//!   refused. Its beta is listed under another bundle ID, which isideload
//!   doesn't set up as AltStore.
//! - Signing registers this iPhone, then runs isideload's `sign_app` with an
//!   app group and without Increased Memory Limit, which AltStore, an app
//!   store that renews its own App ID, doesn't keep. isideload knows AltStore
//!   by its bundle ID and does for it what AltServer does: this iPhone's UDID
//!   goes into its Info.plist (`ALTDeviceID`), the app group into its and its
//!   widget's (`ALTAppGroups`), the signing certificate into the app
//!   (`ALTCertificate.p12`, named by `ALTCertificateID`), so AltStore signs
//!   with the same certificate once you sign in to it, and the RPPairing
//!   record, sealed with the certificate's machine ID, as `ALTPairingFile.dat`,
//!   where AltStore 2.3 read it. 2.3.1 and later read the copy step 7 writes.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use isideload::sideload::sideloader::MemoryLimit;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::account::Session;
use crate::fail::{Fail, Result};
use crate::tunnel::DeviceInfo;
use crate::{ui, Args, VERSION};

/// AltStore's own source: the list of apps and versions AltStore reads, and
/// AltServer installs AltStore from. It redirects to cdn.altstore.io.
pub const SOURCE_URL: &str = "https://apps.altstore.io";
/// What messages call the source.
pub const SOURCE_HOST: &str = "apps.altstore.io";

/// AltStore Classic's bundle ID as published, before signing adds the team
/// ID: the one isideload sets up as AltStore.
pub const ALTSTORE_BUNDLE_ID: &str = "com.rileytestut.AltStore";

/// The first version with Remote AltServer, which installs and refreshes apps
/// without a computer. Earlier ones need AltServer on a computer for
/// everything, so the source's are left out (`AltSourceFeed.oldestVersion`).
const OLDEST_VERSION: [u32; 2] = [2, 3];

/// In AltStore builds that take the pairing from their Documents, where step 7
/// writes it: 2.3.1 and later (altstoreio/AltStore 5811912).
const DOCUMENTS_PAIRING_MARKER: &[u8] = b"ALTPairingFile.plist";

/// An Info.plist is kilobytes; anything far bigger isn't one.
const MAX_INFO_PLIST: u64 = 4 << 20;
/// AltStore's source is about 120 KB; anything far bigger isn't one.
const MAX_LISTING: usize = 8 << 20;
/// How long the source may take to answer.
const LISTING_WAIT: Duration = Duration::from_secs(30);

/// Where the download is kept, in `~/.altboot/downloads`.
const DOWNLOAD_NAME: &str = "AltStore.ipa";

/// A version of AltStore that its source lists and this iPhone can run.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    /// As listed: "2.3.3".
    pub version: String,
    pub url: String,
    /// The SHA-256 listed for the IPA, in lowercase hex. None when the listing
    /// gives none, or something that isn't one.
    pub sha256: Option<String>,
}

/// What the IPA is, read before signing it.
#[derive(Debug, PartialEq)]
pub struct Build {
    /// `CFBundleShortVersionString`, when it has one.
    pub version: Option<String>,
    /// It takes the pairing from its Documents (`ALTPairingFile.plist`), as
    /// AltStore does from 2.3.1 on. 2.3 reads only the copy sealed into it.
    pub reads_documents_pairing: bool,
}

impl Build {
    /// 2.3 or later: it can set up Remote AltServer.
    pub fn has_remote_altserver(&self) -> bool {
        self.version.as_deref().is_some_and(offered)
    }
}

pub struct Signed {
    /// The signed `.app` folder, ready to upload.
    pub app_dir: PathBuf,
    pub bundle_id: String,
    pub name: String,
    /// `CFBundleShortVersionString`, when it has one.
    pub version: Option<String>,
}

/// `--ipa` as a path, or the URL it names, or the newest AltStore its source
/// lists for this iPhone's iOS.
pub async fn fetch_ipa(args: &Args, device: &DeviceInfo) -> Result<PathBuf> {
    if let Some(source) = args.ipa.as_deref() {
        if !source.starts_with("https://") && !source.starts_with("http://") {
            let path = PathBuf::from(source);
            check_ipa(&path)?;
            ui::ok(&format!("using {}", path.display()));
            return Ok(path);
        }
    }
    let dir = args.home.join("downloads");
    std::fs::create_dir_all(&dir)
        .map_err(|e| Fail::new(format!("Couldn't create {}.", dir.display()), e.to_string()))?;
    let dest = dir.join(DOWNLOAD_NAME);
    let client = reqwest::Client::builder()
        .user_agent(format!("altboot/{VERSION}"))
        .build()
        .map_err(|e| Fail::new("Couldn't start a download.", e.to_string()))?;
    if let Some(url) = args.ipa.as_deref() {
        download(&client, url, &dest, "AltStore", None).await?;
        return Ok(dest);
    }

    let newest = newest_listed(&client, &device.ios_version).await?;
    ui::ok(&format!(
        "AltStore {} is the newest for iOS {} on {SOURCE_HOST}",
        newest.version, device.ios_version
    ));
    ui::detail(&newest.url);
    let what = format!("AltStore {}", newest.version);
    match &newest.sha256 {
        Some(sha256) => {
            // Forty megabytes aren't fetched again while the copy from an
            // earlier run is still the file the source lists.
            if dest.exists() {
                ui::activity("checking the AltStore downloaded on an earlier run");
                if sha256_of(&dest).ok().as_deref() == Some(sha256.as_str()) {
                    ui::ok(&format!(
                        "the {what} downloaded on an earlier run is the file {SOURCE_HOST} lists"
                    ));
                    return Ok(dest);
                }
            }
        }
        None => ui::warn(&format!(
            "{SOURCE_HOST} gives no SHA-256 for {what}, so the download is only checked to be \
             an IPA."
        )),
    }
    download(&client, &newest.url, &dest, &what, newest.sha256.as_deref()).await?;
    Ok(dest)
}

/// The first version AltStore's source lists that this iPhone can run.
async fn newest_listed(client: &reqwest::Client, ios: &str) -> Result<Listed> {
    ui::activity(&format!("asking {SOURCE_HOST} for the newest AltStore"));
    ui::detail(SOURCE_URL);
    let unreachable = |raw: String| {
        Fail::new(
            format!(
                "Couldn't reach {SOURCE_HOST}, AltStore's source. Check the internet connection \
                 (LocalDevVPN only routes 10.7.0.1, so it isn't the cause)."
            ),
            raw,
        )
    };
    let response = client
        .get(SOURCE_URL)
        .timeout(LISTING_WAIT)
        .send()
        .await
        .map_err(|e| unreachable(crate::error_text::error_text(&e)))?;
    if !response.status().is_success() {
        return Err(Fail::new(
            format!(
                "{SOURCE_HOST} answered {} for AltStore's source. Try again in a while.",
                response.status()
            ),
            response.url().to_string(),
        ));
    }
    if response.content_length().is_some_and(|n| n > MAX_LISTING as u64) {
        return Err(Fail::new(
            format!("{SOURCE_HOST} sent something far bigger than AltStore's source."),
            format!("{:?} bytes", response.content_length()),
        ));
    }
    let listing = response
        .bytes()
        .await
        .map_err(|e| unreachable(crate::error_text::error_text(&e)))?;
    let versions = listed_versions(&listing, ios).map_err(|raw| {
        Fail::new(
            format!("{SOURCE_HOST} didn't list AltStore as AltStore's source does. Try again in a while."),
            raw,
        )
    })?;
    versions.into_iter().next().ok_or_else(|| {
        Fail::new(
            format!("{SOURCE_HOST} lists no AltStore from 2.3 on that runs on iOS {ios}."),
            format!("no listed {ALTSTORE_BUNDLE_ID} version for iOS {ios}"),
        )
    })
}

/// One version as AltStore's source lists it. Only what the download reads.
#[derive(Deserialize)]
struct ListedVersion {
    version: String,
    #[serde(rename = "downloadURL")]
    download_url: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default, rename = "minOSVersion")]
    min_os: Option<String>,
    #[serde(default, rename = "maxOSVersion")]
    max_os: Option<String>,
}

/// AltStore Classic's versions in `listing`, an AltStore source's JSON, from
/// 2.3 on, downloaded over HTTPS, that run on iOS `ios`, newest first as
/// listed. As SideInstaller's `listedVersions`, except that each version is
/// read on its own, so one this doesn't understand leaves out only itself.
/// An iOS version that can't be read rules nothing out.
pub fn listed_versions(listing: &[u8], ios: &str) -> std::result::Result<Vec<Listed>, String> {
    #[derive(Deserialize)]
    struct Source {
        apps: Vec<serde_json::Value>,
    }
    let source: Source =
        serde_json::from_slice(listing).map_err(|e| format!("not an AltStore source: {e}"))?;
    let app = source
        .apps
        .iter()
        .find(|app| {
            app.get("bundleIdentifier").and_then(|id| id.as_str()) == Some(ALTSTORE_BUNDLE_ID)
        })
        .ok_or_else(|| format!("{ALTSTORE_BUNDLE_ID} isn't listed"))?;
    let os = os_version(ios);
    let versions = app.get("versions").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(versions
        .into_iter()
        .filter_map(|v| serde_json::from_value::<ListedVersion>(v).ok())
        .filter(|v| offered(&v.version) && v.download_url.starts_with("https://"))
        .filter(|v| os.is_none_or(|os| runs_on(os, v.min_os.as_deref(), v.max_os.as_deref())))
        .map(|v| Listed {
            sha256: v
                .sha256
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            version: v.version,
            url: v.download_url,
        })
        .collect())
}

/// True for a version from 2.3 on. One whose numbers can't all be read
/// ("2.3rc") counts by the numbers it starts with.
fn offered(version: &str) -> bool {
    let parts: Vec<u32> = version
        .split('.')
        .map(|part| part.chars().take_while(char::is_ascii_digit).collect::<String>())
        .take_while(|digits| !digits.is_empty())
        .filter_map(|digits| digits.parse().ok())
        .collect();
    if parts.is_empty() {
        return false;
    }
    for i in 0..parts.len().max(OLDEST_VERSION.len()) {
        let have = parts.get(i).copied().unwrap_or(0);
        let need = OLDEST_VERSION.get(i).copied().unwrap_or(0);
        if have != need {
            return have > need;
        }
    }
    true
}

/// "27.1" or "26.0.1" as numbers; None for anything else.
fn os_version(text: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<Option<u32>> = text.trim().split('.').map(|p| p.parse().ok()).collect();
    if !(1..=3).contains(&parts.len()) || parts.iter().any(Option::is_none) {
        return None;
    }
    let n = |i: usize| parts.get(i).copied().flatten().unwrap_or(0);
    Some((n(0), n(1), n(2)))
}

/// True when iOS `os` is in a listing's range: `min` or later, and `max` or
/// earlier when it gives one. A bound that isn't a version is no bound.
fn runs_on(os: (u32, u32, u32), min: Option<&str>, max: Option<&str>) -> bool {
    if min.and_then(os_version).is_some_and(|min| os < min) {
        return false;
    }
    if max.and_then(os_version).is_some_and(|max| os > max) {
        return false;
    }
    true
}

/// Downloads `url` to `dest`, keeping it only if it's an IPA and, when
/// `sha256` is given, that file. `what` is what's downloaded, for the screen.
async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    what: &str,
    sha256: Option<&str>,
) -> Result<()> {
    let host = host_of(url);
    ui::status(&format!("downloading {what}"));
    ui::detail(url);
    let mut response = client.get(url).send().await.map_err(|e| {
        Fail::new(
            format!(
                "Couldn't download {what} from {host}. Check the internet connection \
                 (LocalDevVPN only routes 10.7.0.1, so it isn't the cause)."
            ),
            crate::error_text::error_text(&e),
        )
    })?;
    if !response.status().is_success() {
        return Err(Fail::new(
            format!(
                "{host} answered {} for the {what} download. Try again in a while.",
                response.status()
            ),
            url.to_string(),
        ));
    }
    let total = response.content_length();
    let partial = dest.with_extension("ipa.part");
    let mut file = std::fs::File::create(&partial).map_err(|e| {
        Fail::new(format!("Couldn't write {}.", partial.display()), e.to_string())
    })?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    loop {
        let chunk = response.chunk().await.map_err(|e| {
            Fail::new(format!("The {what} download broke off. Run altboot again."), e.to_string())
        })?;
        let Some(chunk) = chunk else { break };
        file.write_all(&chunk).map_err(|e| {
            Fail::new(format!("Couldn't write {}.", partial.display()), e.to_string())
        })?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            ui::progress("downloaded", received, total);
        }
    }
    ui::progress_end();
    drop(file);
    let got = hex::encode(hasher.finalize());
    if let Some(expected) = sha256 {
        if got != expected {
            let _ = std::fs::remove_file(&partial);
            return Err(Fail::new(
                format!(
                    "The download isn't the file {SOURCE_HOST} lists for {what}, so altboot \
                     won't install it. Run altboot again in a while."
                ),
                format!("SHA-256 {got}, listed {expected}"),
            ));
        }
    }
    std::fs::rename(&partial, dest)
        .map_err(|e| Fail::new(format!("Couldn't write {}.", dest.display()), e.to_string()))?;
    check_ipa(dest)?;
    let size = format!("{:.1} MB", received as f64 / 1_048_576.0);
    match sha256 {
        Some(expected) => {
            ui::ok(&format!("downloaded {what} ({size}), the file {SOURCE_HOST} lists"));
            ui::detail(&format!("SHA-256 {expected}"));
        }
        None => ui::ok(&format!("downloaded {what} ({size})")),
    }
    Ok(())
}

/// The SHA-256 of the file at `path`, in lowercase hex.
fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The host of a URL ("https://host/..."), or the URL itself.
pub fn host_of(url: &str) -> &str {
    url.split_once("://")
        .and_then(|(_, rest)| rest.split(['/', ':', '?', '#']).next())
        .filter(|host| !host.is_empty())
        .unwrap_or(url)
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
                "{} isn't an IPA. The server may have sent an error page: try again in a while.",
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

/// Reads the IPA's app before it's signed: that it's AltStore Classic, since
/// signing spends one of the week's App IDs, and what this build reads.
pub fn inspect(ipa: &Path) -> Result<Build> {
    ui::activity("checking it's AltStore");
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
    if bundle_id != ALTSTORE_BUNDLE_ID {
        return Err(Fail::new(
            format!(
                "{} isn't AltStore Classic: its app is {}. altboot installs AltStore Classic only \
                 ({ALTSTORE_BUNDLE_ID}), as AltStore's source publishes it.",
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
        reads_documents_pairing: memchr::memmem::find(&binary, DOCUMENTS_PAIRING_MARKER).is_some(),
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

/// Signs AltStore for `device`. `pairing` is its RPPairing record, which
/// isideload seals into the app for AltStore 2.3; None leaves it out.
pub async fn sign(
    session: &mut Session,
    ipa: &Path,
    device: &DeviceInfo,
    pairing: Option<&[u8]>,
) -> Result<Signed> {
    ui::status("signing; this takes a while here");
    // SideInstaller asks for Increased Memory Limit where it's safe, which
    // isideload turns off for AltStore anyway: AltStore registers its App ID
    // again each week, and a free team's profile then needn't grant it.
    let (app_dir, _special) = session
        .sideloader
        .sign_app(
            ipa.to_path_buf(),
            None,
            MemoryLimit::Off,
            true,
            Some((device.name.as_str(), device.udid.as_str())),
            pairing,
        )
        .await
        .map_err(|e| {
            let text = crate::error_text::report_text(&e);
            Fail::new(sign_advice(&text), text)
        })?;
    let info = bundle_info(&app_dir)?;
    ui::ok(&format!("signed {} as {}", info.name, info.bundle_id));
    // What isideload set up, as AltServer would have.
    if info.device_id.as_deref() == Some(device.udid.as_str()) {
        ui::detail("ALTDeviceID is this iPhone's UDID");
    } else {
        ui::warn(
            "AltStore's Info.plist doesn't name this iPhone, so AltStore may register another \
             device with your team and sign apps for it.",
        );
        ui::detail(&format!("ALTDeviceID {:?}", info.device_id));
    }
    ui::detail(&match &info.certificate_id {
        Some(serial) => format!("ALTCertificateID {serial}, with ALTCertificate.p12 beside it"),
        None => "no ALTCertificateID".to_string(),
    });
    if pairing.is_some() {
        ui::detail(&format!(
            "ALTPairingFile.dat sealed into it: {}",
            if app_dir.join("ALTPairingFile.dat").exists() { "yes" } else { "no" }
        ));
    }
    Ok(Signed {
        app_dir,
        bundle_id: info.bundle_id,
        name: info.name,
        version: info.version,
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

/// What the signed app's Info.plist says.
#[derive(Debug)]
struct BundleInfo {
    bundle_id: String,
    /// Its Home Screen name.
    name: String,
    version: Option<String>,
    /// `ALTDeviceID`: the iPhone AltStore signs apps for.
    device_id: Option<String>,
    /// `ALTCertificateID`: the certificate AltStore signs with.
    certificate_id: Option<String>,
}

fn bundle_info(app_dir: &Path) -> Result<BundleInfo> {
    let info: plist::Dictionary = plist::from_file(app_dir.join("Info.plist"))
        .map_err(|e| Fail::new("The signed app has no readable Info.plist.", e.to_string()))?;
    let text = |key: &str| {
        info.get(key)
            .and_then(|v| v.as_string())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let bundle_id = text("CFBundleIdentifier").ok_or_else(|| {
        Fail::new(
            "The signed app has no bundle identifier.",
            "CFBundleIdentifier missing",
        )
    })?;
    Ok(BundleInfo {
        bundle_id,
        name: text("CFBundleDisplayName")
            .or_else(|| text("CFBundleName"))
            .unwrap_or_else(|| "AltStore".into()),
        version: text("CFBundleShortVersionString"),
        device_id: text("ALTDeviceID"),
        certificate_id: text("ALTCertificateID"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("altboot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// AltStore's source as it was on 2026-11-14, cut down: AltStore Classic's
    /// 2.3.3 to 2.2.2 and its beta, plus versions made up to be left out.
    fn a_listing() -> Vec<u8> {
        let version = |v: &str, url: &str, sha: &str, min: &str| {
            serde_json::json!({
                "version": v, "date": "2026-11-09T13:00:00-04:00", "downloadURL": url,
                "sha256": sha, "minOSVersion": min, "size": 40615381
            })
        };
        let cdn = |file: &str| format!("https://cdn.altstore.io/file/altstore/apps/altstore/{file}");
        serde_json::to_vec(&serde_json::json!({
            "name": "AltStore",
            "identifier": "com.rileytestut.AltStore",
            "apps": [
                {
                    "bundleIdentifier": "com.rileytestut.AltStore.Beta",
                    "versions": [version("2.4b1", &cdn("2_4_b1.ipa"), &"b".repeat(64), "17.4")]
                },
                {
                    "bundleIdentifier": "com.rileytestut.AltStore",
                    "name": "AltStore",
                    "versions": [
                        version("2.4", &cdn("2_4.ipa"), &"c".repeat(64), "28.0"),
                        {"version": "2.3.4", "date": "2026-11-12"},
                        version("2.3.3", &cdn("2_3_3.ipa"),
                                "3BCFC4EF816ABF1551AB31304040D8D556382657663E1F0B720D1A505CA8ACA9", "17.4"),
                        version("2.3.2", "http://cdn.altstore.io/2_3_2.ipa", &"d".repeat(64), "17.4"),
                        version("2.3.1", &cdn("2_3_1.ipa"), "not a checksum", "17.4"),
                        version("2.3", &cdn("2_3.ipa"), &"e".repeat(64), "17.4"),
                        version("2.2.2", &cdn("2_2_2.ipa"), &"f".repeat(64), "14.0"),
                    ]
                },
                {"bundleIdentifier": "com.rileytestut.Delta", "versions": [{"version": "1.7"}]}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn the_newest_listed_altstore_that_runs_here_comes_first() {
        let versions = listed_versions(&a_listing(), "27.1").unwrap();
        let names: Vec<&str> = versions.iter().map(|v| v.version.as_str()).collect();
        // 2.4 needs iOS 28, 2.3.4 has no download, 2.3.2 isn't over HTTPS, and
        // 2.2.2 has no Remote AltServer.
        assert_eq!(names, ["2.3.3", "2.3.1", "2.3"]);
        assert_eq!(
            versions[0].sha256.as_deref(),
            Some("3bcfc4ef816abf1551ab31304040d8d556382657663e1f0b720d1a505ca8aca9"),
            "lowercased, as the download's is"
        );
        assert_eq!(
            versions[0].url,
            "https://cdn.altstore.io/file/altstore/apps/altstore/2_3_3.ipa"
        );
        assert_eq!(versions[1].sha256, None, "something that isn't a SHA-256 is none");
    }

    #[test]
    fn the_ios_range_decides_what_runs() {
        assert!(listed_versions(&a_listing(), "17.0").unwrap().is_empty());
        let first = |ios: &str| listed_versions(&a_listing(), ios).unwrap()[0].version.clone();
        assert_eq!(first("17.4"), "2.3.3");
        assert_eq!(first("28.0.1"), "2.4");
        // An iOS version that can't be read rules nothing out.
        assert_eq!(first(""), "2.4");
    }

    #[test]
    fn a_listing_without_altstore_or_not_json_is_refused() {
        assert!(listed_versions(b"<html>", "27.1").unwrap_err().contains("not an AltStore source"));
        let others = br#"{"apps": [{"bundleIdentifier": "com.rileytestut.AltStore.Beta"}]}"#;
        assert!(listed_versions(others, "27.1").unwrap_err().contains("isn't listed"));
        let empty = br#"{"apps": [{"bundleIdentifier": "com.rileytestut.AltStore"}]}"#;
        assert!(listed_versions(empty, "27.1").unwrap().is_empty());
    }

    #[test]
    fn versions_from_2_3_on_are_offered() {
        for (version, offered_) in [
            ("2.3", true),
            ("2.3.0", true),
            ("2.3.3", true),
            ("2.3rc", true),
            ("2.10", true),
            ("3.0", true),
            ("2.2.2", false),
            ("2", false),
            ("1.9.9", false),
            ("v2.3", false),
            ("", false),
        ] {
            assert_eq!(offered(version), offered_, "{version}");
        }
    }

    #[test]
    fn ios_versions_and_ranges_read_as_numbers() {
        assert_eq!(os_version("27.1"), Some((27, 1, 0)));
        assert_eq!(os_version("26.0.1"), Some((26, 0, 1)));
        assert_eq!(os_version("27"), Some((27, 0, 0)));
        assert_eq!(os_version("27.1 "), Some((27, 1, 0)));
        for bad in ["", "27.x", "1.2.3.4", "unknown"] {
            assert_eq!(os_version(bad), None, "{bad}");
        }
        assert!(runs_on((27, 1, 0), Some("17.4"), None));
        assert!(!runs_on((17, 3, 9), Some("17.4"), None));
        assert!(runs_on((17, 4, 0), Some("17.4"), Some("17.4")));
        assert!(!runs_on((27, 1, 0), Some("17.4"), Some("26.4")));
        assert!(runs_on((27, 1, 0), Some("later"), Some("")));
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
    fn hosts_come_out_of_urls() {
        assert_eq!(host_of("https://cdn.altstore.io/file/a.ipa"), "cdn.altstore.io");
        assert_eq!(host_of("http://127.0.0.1:8080/a.ipa"), "127.0.0.1");
        assert_eq!(host_of("nonsense"), "nonsense");
    }

    /// Serves `body` once, as an HTTP/1.1 server would, and returns its URL.
    async fn serve_once(body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/AltStore.ipa", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    return;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        });
        url
    }

    #[tokio::test]
    async fn only_the_listed_file_is_kept() {
        let dir = temp_dir("download");
        let dest = dir.join(DOWNLOAD_NAME);
        let ipa = b"PK\x03\x04 an AltStore".to_vec();
        let listed = hex::encode(Sha256::digest(&ipa));
        let client = reqwest::Client::new();

        let url = serve_once(ipa.clone()).await;
        download(&client, &url, &dest, "AltStore 2.3.3", Some(&listed)).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), ipa);
        assert_eq!(sha256_of(&dest).unwrap(), listed, "an earlier run's copy is recognized");

        let other = b"PK\x03\x04 something else".to_vec();
        let url = serve_once(other).await;
        let refused = download(&client, &url, &dest, "AltStore 2.3.3", Some(&listed))
            .await
            .unwrap_err();
        assert!(refused.advice.contains("isn't the file apps.altstore.io lists"), "{}", refused.advice);
        assert!(refused.raw.contains(&listed));
        assert_eq!(std::fs::read(&dest).unwrap(), ipa, "the file before it is left alone");
        assert!(!dest.with_extension("ipa.part").exists(), "nothing of the refused one is kept");
    }

    /// An IPA with `bundle_id`'s Info.plist and an executable holding `binary`,
    /// plus a widget, whose Info.plist mustn't be taken for the app's.
    fn an_ipa(dir: &Path, bundle_id: &str, version: &str, binary: &[u8]) -> PathBuf {
        let path = dir.join(format!("{bundle_id}-{version}.ipa"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let plist = |id: &str, exec: &str| {
            let mut info = plist::Dictionary::new();
            info.insert("CFBundleIdentifier".into(), id.into());
            info.insert("CFBundleExecutable".into(), exec.into());
            info.insert("CFBundleShortVersionString".into(), version.into());
            let mut bytes = Vec::new();
            plist::to_writer_binary(&mut bytes, &info).unwrap();
            bytes
        };
        zip.start_file("Payload/AltStore.app/PlugIns/AltWidgetExtension.appex/Info.plist", options)
            .unwrap();
        zip.write_all(&plist("com.rileytestut.AltStore.AltWidget", "AltWidgetExtension"))
            .unwrap();
        zip.start_file("Payload/AltStore.app/Info.plist", options).unwrap();
        zip.write_all(&plist(bundle_id, "AltStore")).unwrap();
        zip.start_file("Payload/AltStore.app/AltStore", options).unwrap();
        zip.write_all(binary).unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn altstore_2_3_1_on_reads_the_pairing_from_its_documents() {
        let dir = temp_dir("altstore");
        let build = inspect(&an_ipa(&dir, ALTSTORE_BUNDLE_ID, "2.3.3", b"..ALTPairingFile.plist..")).unwrap();
        assert!(build.reads_documents_pairing);
        assert!(build.has_remote_altserver());
        assert_eq!(build.version.as_deref(), Some("2.3.3"));
    }

    #[test]
    fn altstore_2_3_reads_only_the_sealed_copy_and_2_2_has_no_remote_altserver() {
        let dir = temp_dir("older");
        let build = inspect(&an_ipa(&dir, ALTSTORE_BUNDLE_ID, "2.3", b"..ALTPairingFile.dat..")).unwrap();
        assert!(!build.reads_documents_pairing);
        assert!(build.has_remote_altserver());
        let build = inspect(&an_ipa(&dir, ALTSTORE_BUNDLE_ID, "2.2.2", b"")).unwrap();
        assert!(!build.has_remote_altserver());
    }

    #[test]
    fn anything_but_altstore_classic_is_refused_before_signing() {
        let dir = temp_dir("other");
        for id in [
            "com.rileytestut.AltStore.Beta",
            "com.SideStore.SideStore",
            "com.rileytestut.AltStore.ABCDE12345",
        ] {
            let failure = inspect(&an_ipa(&dir, id, "2.3.3", b"")).unwrap_err();
            assert!(failure.advice.contains("isn't AltStore Classic"), "{}", failure.advice);
            assert!(failure.advice.contains(id), "{}", failure.advice);
        }
    }

    #[test]
    fn the_apps_own_info_plist_is_found_among_its_extensions() {
        let names: Vec<String> = [
            "Payload/",
            "Payload/AltStore.app/PlugIns/AltWidgetExtension.appex/Info.plist",
            "Payload/AltStore.app/Frameworks/AltStoreCore.framework/Info.plist",
            "Payload/AltStore.app/Info.plist",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            main_info_plist(&names).as_deref(),
            Some("Payload/AltStore.app/Info.plist")
        );
        assert_eq!(main_info_plist(&names[..3]), None);
    }

    #[test]
    fn the_signed_apps_info_plist_says_what_altstore_reads() {
        let dir = temp_dir("app").join("AltStore.app");
        std::fs::create_dir_all(&dir).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert(
            "CFBundleIdentifier".into(),
            "com.rileytestut.AltStore.ABCDE12345".into(),
        );
        info.insert("CFBundleName".into(), "AltStore".into());
        info.insert("CFBundleShortVersionString".into(), "2.3.3".into());
        info.insert("ALTDeviceID".into(), "00008150-0001".into());
        plist::to_file_xml(dir.join("Info.plist"), &info).unwrap();
        let read = bundle_info(&dir).unwrap();
        assert_eq!(read.bundle_id, "com.rileytestut.AltStore.ABCDE12345");
        assert_eq!(read.name, "AltStore");
        assert_eq!(read.version.as_deref(), Some("2.3.3"));
        assert_eq!(read.device_id.as_deref(), Some("00008150-0001"));
        assert_eq!(read.certificate_id, None);
    }

    /// Step 5's read of a real AltStore IPA, named by `ALTBOOT_ALTSTORE_IPA`:
    /// `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_real_altstore_ipa_reads_as_altstore() {
        let path = std::env::var_os("ALTBOOT_ALTSTORE_IPA")
            .expect("ALTBOOT_ALTSTORE_IPA names an AltStore IPA");
        let build = inspect(Path::new(&path)).unwrap();
        println!("{build:?}");
        assert!(build.has_remote_altserver());
    }

    /// Step 5's download from AltStore's live source, run twice: the second
    /// keeps the first one's file. `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn the_live_source_downloads_altstore_once() {
        let home = temp_dir("live");
        let argv = ["--home".to_string(), home.display().to_string()];
        let crate::Parsed::Run(args) = crate::parse_args(argv).unwrap() else {
            panic!("not a run");
        };
        let device = DeviceInfo {
            name: "iPhone".into(),
            product_type: "iPhone17,3".into(),
            ios_version: "27.1".into(),
            udid: "00008150-0001".into(),
        };
        let ipa = fetch_ipa(&args, &device).await.unwrap();
        let build = inspect(&ipa).unwrap();
        println!("{build:?}");
        assert!(build.has_remote_altserver() && build.reads_documents_pairing);
        let modified = std::fs::metadata(&ipa).unwrap().modified().unwrap();
        assert_eq!(fetch_ipa(&args, &device).await.unwrap(), ipa);
        let again = std::fs::metadata(&ipa).unwrap().modified().unwrap();
        assert_eq!(again, modified, "not downloaded again");
    }

    /// AltStore's live source, read as step 5 reads it: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn the_live_source_lists_an_altstore_for_ios_27() {
        let listing = reqwest::get(SOURCE_URL).await.unwrap().bytes().await.unwrap();
        let versions = listed_versions(&listing, "27.1").unwrap();
        println!("{versions:#?}");
        assert!(versions[0].sha256.is_some());
    }
}
