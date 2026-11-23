//! Step 5: sign the IPA step 1 got (download.rs) for this iPhone, as Side by
//! Side does for another one: register the device, then isideload's
//! `sign_app` with the app group and no Increased Memory Limit
//! (`SideBySideView.signApp`).

use std::path::{Path, PathBuf};

use isideload::sideload::sideloader::MemoryLimit;

use crate::account::Session;
use crate::fail::{Fail, Result};
use crate::tunnel::DeviceInfo;
use crate::ui;

pub struct Signed {
    /// The signed `.app` folder, ready to upload.
    pub app_dir: PathBuf,
    pub bundle_id: String,
    pub name: String,
    /// `CFBundleShortVersionString`, when it has one.
    pub version: Option<String>,
}

pub async fn sign(session: &mut Session, ipa: &Path, device: &DeviceInfo) -> Result<Signed> {
    ui::status("signing; this takes a while here");
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
    let (bundle_id, name, version) = bundle_info(&app_dir)?;
    ui::ok(&format!("signed {name} as {bundle_id}"));
    Ok(Signed {
        app_dir,
        bundle_id,
        name,
        version,
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

/// Bundle id, Home Screen name and version of the signed app.
fn bundle_info(app_dir: &Path) -> Result<(String, String, Option<String>)> {
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
        .unwrap_or_else(|| "SideInstaller".into());
    let version = text("CFBundleShortVersionString").filter(|v| !v.is_empty());
    Ok((bundle_id, name, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_id_and_name_come_from_info_plist() {
        let dir = std::env::temp_dir().join(format!(
            "siboot-app-{}/SideInstaller.app",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert(
            "CFBundleIdentifier".into(),
            "com.frizzlem.sideinstaller.ABCDE12345".into(),
        );
        info.insert("CFBundleName".into(), "SideInstaller".into());
        plist::to_file_xml(dir.join("Info.plist"), &info).unwrap();
        let (id, name, version) = bundle_info(&dir).unwrap();
        assert_eq!(id, "com.frizzlem.sideinstaller.ABCDE12345");
        assert_eq!(name, "SideInstaller");
        assert_eq!(version, None);
        info.insert("CFBundleShortVersionString".into(), "1.7.0".into());
        plist::to_file_xml(dir.join("Info.plist"), &info).unwrap();
        assert_eq!(bundle_info(&dir).unwrap().2.as_deref(), Some("1.7.0"));
    }
}
