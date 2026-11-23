//! SideInstaller's IPA, got in step 1, before siboot needs LocalDevVPN.
//!
//! iOS runs one VPN at a time, and from step 1's dial of the iPhone on it has
//! to be LocalDevVPN, which carries nothing but 10.7.0.1. Where GitHub is
//! blocked (raw.githubusercontent.com and release downloads, in mainland China
//! and Iran), it's reached only through another VPN, which can't be on then.
//! So siboot gets SideInstaller first, over whatever VPN is on, and step 1
//! then asks for LocalDevVPN when nothing answers at 10.7.0.1 (preflight.rs).
//!
//! The release is kept in ~/.siboot/downloads, with the tag it came from in
//! SideInstaller.json. A later run asks GitHub only which release is the
//! latest (one redirect, no download) and downloads only a newer one. When
//! GitHub can't be reached, siboot installs the kept copy and says which it
//! is.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::header::LOCATION;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fail::{Fail, Result};
use crate::{ui, Args, VERSION};

/// The release Side by Side installs (`SideBySideView.releaseIPA`). GitHub
/// answers it with a redirect to the latest release's own download.
pub const RELEASE_IPA: &str =
    "https://github.com/FrizzleM/SideInstaller/releases/latest/download/SideInstaller.ipa";

/// The kept release, and what it is.
const KEPT_IPA: &str = "SideInstaller.ipa";
const KEPT_INFO: &str = "SideInstaller.json";
/// An IPA from `--ipa URL`: downloaded on every run, never taken for the kept
/// release.
const OTHER_IPA: &str = "other.ipa";

/// Where GitHub is blocked, a connection to it hangs rather than fails.
const CONNECT: Duration = Duration::from_secs(20);
/// Asking which release is the latest is one small answer.
const LOOKUP: Duration = Duration::from_secs(20);
/// A download that sends nothing for this long has stalled.
const STALL: Duration = Duration::from_secs(30);

/// How to get SideInstaller where GitHub is blocked. Apple's sign-in server
/// answered before this is said, so the internet itself works.
const WHERE_BLOCKED: &str =
    "Where GitHub is blocked, turn on a VPN that reaches it and run siboot again: it gets \
     SideInstaller first, then asks you to switch to LocalDevVPN, and keeps it for the runs \
     after. Or put SideInstaller.ipa in a folder you mount in Minis (Settings › Mount External \
     Folders) and run siboot --ipa /var/minis/mounts/<folder>/SideInstaller.ipa.";

/// What step 1 does about the IPA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Have it ready for step 5: downloaded now, or kept from before.
    Get,
    /// `siboot check`: only see that it could be, downloading nothing.
    Look,
}

/// The IPA step 5 signs.
#[derive(Debug)]
pub struct Ipa {
    pub path: PathBuf,
    /// SideInstaller's release, as GitHub tags it ("v2.1.1"). None for one
    /// from `--ipa`.
    pub release: Option<String>,
    /// GitHub answered in this run. Where it's blocked, that means a VPN
    /// other than LocalDevVPN is on now.
    pub github: bool,
}

impl Ipa {
    /// "SideInstaller 2.1.1", for step 1's summary.
    pub fn name(&self) -> String {
        match &self.release {
            Some(tag) => format!("SideInstaller {}", version(tag)),
            None => "your IPA".into(),
        }
    }
}

/// What the kept release is. Written once its download is whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    /// GitHub's tag for it.
    release: String,
    size: u64,
    /// For the log: it can be checked against the release's own digest.
    sha256: String,
}

/// `--ipa` as a path, or the URL it names, or SideInstaller's latest release.
pub async fn get(args: &Args, mode: Mode) -> Result<Ipa> {
    get_from(args.ipa.as_deref(), &args.home, mode, RELEASE_IPA).await
}

/// `get`, with GitHub's address for the latest release given.
async fn get_from(ipa: Option<&str>, home: &Path, mode: Mode, latest: &str) -> Result<Ipa> {
    match ipa {
        Some(path) if !is_url(path) => {
            let path = PathBuf::from(path);
            check_ipa(&path)?;
            ui::ok(&format!("using {}", path.display()));
            Ok(Ipa {
                path,
                release: None,
                github: false,
            })
        }
        Some(url) => from_url(url, &downloads(home)?, mode).await,
        None => release(&downloads(home)?, mode, latest).await,
    }
}

fn is_url(source: &str) -> bool {
    source.starts_with("https://") || source.starts_with("http://")
}

fn downloads(home: &Path) -> Result<PathBuf> {
    let dir = home.join("downloads");
    std::fs::create_dir_all(&dir)
        .map_err(|e| Fail::new(format!("Couldn't create {}.", dir.display()), e.to_string()))?;
    Ok(dir)
}

/// SideInstaller's latest release: kept already, downloaded now, or, when
/// GitHub can't give it, the one kept from an earlier run.
async fn release(dir: &Path, mode: Mode, latest: &str) -> Result<Ipa> {
    let path = dir.join(KEPT_IPA);
    let ipa = |release: String, github: bool| Ipa {
        path: path.clone(),
        release: Some(release),
        github,
    };
    let kept = kept(dir);
    if let Some(k) = &kept {
        ui::detail(&format!(
            "kept from an earlier run: {} ({} bytes, SHA-256 {})",
            k.release, k.size, k.sha256
        ));
    }

    // What becomes of the kept copy when GitHub can't give a newer one.
    let installs = match mode {
        Mode::Get => "siboot installs",
        Mode::Look => "a run would install",
    };

    ui::activity("asking GitHub for SideInstaller's latest release");
    let (tag, url) = match latest_release(latest).await {
        Ok(found) => found,
        Err(raw) => {
            let Some(k) = kept else {
                return Err(Fail::new(
                    format!(
                        "Couldn't reach GitHub, where siboot gets SideInstaller. {WHERE_BLOCKED}"
                    ),
                    raw,
                ));
            };
            ui::warn(&format!(
                "Couldn't reach GitHub, so {installs} SideInstaller {}, downloaded on an earlier \
                 run.",
                version(&k.release)
            ));
            ui::detail(&raw);
            return Ok(ipa(k.release, false));
        }
    };
    ui::detail(&format!("the latest release is {tag}"));
    if kept.as_ref().is_some_and(|k| k.release == tag) {
        ui::ok(&format!(
            "SideInstaller {}, the latest, is downloaded already",
            version(&tag)
        ));
        return Ok(ipa(tag, true));
    }

    let got = match mode {
        Mode::Look => {
            ui::activity("reaching GitHub's download");
            reachable(&url).await.map(|()| {
                ui::ok(&format!(
                    "GitHub answers, and SideInstaller {} is there to download",
                    version(&tag)
                ))
            })
        }
        Mode::Get => {
            ui::status(&format!("downloading SideInstaller {}", version(&tag)));
            download(&url, &path).await.map(|got| {
                let kept = Kept {
                    release: tag.clone(),
                    size: got.size,
                    sha256: got.sha256,
                };
                if let Err(e) = keep(dir, &kept) {
                    ui::detail(&format!("couldn't write {KEPT_INFO}: {e}"));
                }
                ui::ok(&format!(
                    "downloaded SideInstaller {} ({})",
                    version(&tag),
                    megabytes(got.size)
                ));
            })
        }
    };
    let raw = match got {
        Ok(()) => return Ok(ipa(tag, true)),
        Err(raw) => raw,
    };
    let missed = match mode {
        Mode::Get => format!("download SideInstaller {} from GitHub", version(&tag)),
        Mode::Look => format!("reach SideInstaller {}'s download on GitHub", version(&tag)),
    };
    let Some(k) = kept else {
        return Err(Fail::new(
            format!("Couldn't {missed}. Try again in a minute. {WHERE_BLOCKED}"),
            raw,
        ));
    };
    ui::warn(&format!(
        "Couldn't {missed}, so {installs} {}, downloaded on an earlier run.",
        version(&k.release)
    ));
    ui::detail(&raw);
    Ok(ipa(k.release, true))
}

/// An IPA from `--ipa URL`, downloaded again on every run.
async fn from_url(url: &str, dir: &Path, mode: Mode) -> Result<Ipa> {
    let host = host_of(url);
    let got = match mode {
        Mode::Look => {
            ui::activity(&format!("reaching {host}"));
            reachable(url)
                .await
                .map(|()| ui::ok(&format!("{host} answers, and the IPA is there")))
        }
        Mode::Get => {
            ui::status(&format!("downloading the IPA from {host}"));
            download(url, &dir.join(OTHER_IPA))
                .await
                .map(|got| ui::ok(&format!("downloaded the IPA ({})", megabytes(got.size))))
        }
    };
    got.map_err(|raw| {
        Fail::new(
            format!(
                "Couldn't download the IPA from {host}. Check the address and the internet \
                 connection."
            ),
            raw,
        )
    })?;
    Ok(Ipa {
        path: dir.join(OTHER_IPA),
        release: None,
        github: false,
    })
}

/// The kept release, while its file is still the size its description says
/// and an IPA.
fn kept(dir: &Path) -> Option<Kept> {
    let text = std::fs::read_to_string(dir.join(KEPT_INFO)).ok()?;
    let kept: Kept = serde_json::from_str(&text).ok()?;
    let ipa = dir.join(KEPT_IPA);
    let size = std::fs::metadata(&ipa).ok()?.len();
    (size == kept.size && is_zip(&ipa).unwrap_or(false)).then_some(kept)
}

/// Describe the kept release. Through a temporary file, so a description is
/// always whole.
fn keep(dir: &Path, kept: &Kept) -> std::io::Result<()> {
    let path = dir.join(KEPT_INFO);
    let part = dir.join(format!("{KEPT_INFO}.part"));
    let text = serde_json::to_vec_pretty(kept).map_err(std::io::Error::other)?;
    std::fs::write(&part, text)?;
    std::fs::rename(&part, &path)
}

/// The latest release's tag and download URL, from the redirect GitHub
/// answers `latest` with: nothing downloaded, and no API, which is
/// rate-limited and another host to be blocked.
async fn latest_release(latest: &str) -> std::result::Result<(String, String), String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("siboot/{VERSION}"))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT)
        .timeout(LOOKUP)
        .build()
        .map_err(|e| crate::error_text::error_text(&e))?;
    let response = client
        .head(latest)
        .send()
        .await
        .map_err(|e| crate::error_text::error_text(&e))?;
    let status = response.status();
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|v| v.to_str().ok());
    let (true, Some(location)) = (status.is_redirection(), location) else {
        return Err(format!(
            "{latest} answered {status}, not a redirect to a release"
        ));
    };
    let url = response
        .url()
        .join(location)
        .map_err(|e| format!("{location}: {e}"))?;
    let tag = tag_of(url.as_str()).ok_or_else(|| format!("no release tag in {url}"))?;
    Ok((tag, url.to_string()))
}

/// "v2.1.1" in ".../releases/download/v2.1.1/SideInstaller.ipa".
fn tag_of(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let mut parts = path.rsplit('/');
    parts.next().filter(|file| !file.is_empty())?;
    let tag = parts.next().filter(|tag| !tag.is_empty())?;
    (parts.next()? == "download").then(|| tag.to_string())
}

/// "2.1.1" for the tag "v2.1.1".
fn version(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string())
}

fn download_client() -> std::result::Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("siboot/{VERSION}"))
        .connect_timeout(CONNECT)
        .read_timeout(STALL)
        .build()
        .map_err(|e| crate::error_text::error_text(&e))
}

/// The download answers, without downloading it.
async fn reachable(url: &str) -> std::result::Result<(), String> {
    let response = download_client()?
        .head(url)
        .send()
        .await
        .map_err(|e| crate::error_text::error_text(&e))?;
    match response.status() {
        status if status.is_success() => Ok(()),
        status => Err(format!(
            "{} answered {status}",
            host_of(response.url().as_str())
        )),
    }
}

/// What a whole download came to.
struct Got {
    size: u64,
    sha256: String,
}

/// `url` into `dest`, through `dest.part`, so `dest` only ever holds a whole
/// IPA and an earlier one stays until a new one has arrived. The error is the
/// raw cause, for the caller's advice.
async fn download(url: &str, dest: &Path) -> std::result::Result<Got, String> {
    ui::detail(url);
    let mut response = download_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| crate::error_text::error_text(&e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "{} answered {status}",
            host_of(response.url().as_str())
        ));
    }
    let total = response.content_length().filter(|t| *t > 0);
    let partial = dest.with_extension("ipa.part");
    let mut file = std::fs::File::create(&partial)
        .map_err(|e| format!("couldn't write {}: {e}", partial.display()))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let arrived = loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if let Err(e) = file.write_all(&chunk) {
                    break Err(format!("couldn't write {}: {e}", partial.display()));
                }
                hasher.update(&chunk);
                received += chunk.len() as u64;
                if let Some(total) = total {
                    ui::progress("downloaded", received, total);
                }
            }
            Ok(None) => break Ok(()),
            Err(e) => {
                break Err(format!(
                    "the download broke off: {}",
                    crate::error_text::error_text(&e)
                ))
            }
        }
    };
    ui::progress_end();
    drop(file);
    let whole = arrived
        .and_then(|()| match total {
            Some(total) if total != received => Err(format!(
                "the download stopped at {received} of {total} bytes"
            )),
            _ => Ok(()),
        })
        .and_then(|()| match is_zip(&partial) {
            Ok(true) => Ok(()),
            Ok(false) => Err(format!(
                "what arrived from {} isn't an IPA (an error page, say)",
                host_of(response.url().as_str())
            )),
            Err(e) => Err(format!("couldn't read {}: {e}", partial.display())),
        })
        .and_then(|()| {
            std::fs::rename(&partial, dest)
                .map_err(|e| format!("couldn't write {}: {e}", dest.display()))
        });
    if whole.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    whole.map(|()| Got {
        size: received,
        sha256: hex::encode(hasher.finalize()),
    })
}

/// An IPA is a zip: an error page or a truncated download isn't, and would
/// only fail later, inside the signer.
fn is_zip(path: &Path) -> std::io::Result<bool> {
    let mut magic = [0u8; 4];
    let mut file = std::fs::File::open(path)?;
    match std::io::Read::read_exact(&mut file, &mut magic) {
        Ok(()) => Ok(&magic == b"PK\x03\x04"),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

/// `--ipa PATH` names an IPA.
fn check_ipa(path: &Path) -> Result<()> {
    match is_zip(path) {
        Ok(true) => Ok(()),
        Ok(false) => Err(Fail::new(
            format!("{} isn't an IPA.", path.display()),
            "it doesn't start as a zip does",
        )),
        Err(e) => Err(Fail::new(
            format!("Couldn't read {}.", path.display()),
            e.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// An IPA as far as siboot looks: a zip's first bytes, then `tag` so two
    /// releases differ.
    fn ipa_bytes(tag: &str) -> Vec<u8> {
        let mut bytes = b"PK\x03\x04".to_vec();
        bytes.extend(format!("SideInstaller {tag} ").repeat(500).into_bytes());
        bytes
    }

    fn home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "siboot-test-download-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    /// One answer: status line and headers, then the body. `short` sends that
    /// many bytes fewer than Content-Length says, then hangs up.
    struct Answer {
        head: String,
        body: Vec<u8>,
        short: usize,
    }

    fn redirect(to: &str) -> Answer {
        Answer {
            head: format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n"),
            body: Vec::new(),
            short: 0,
        }
    }

    fn file(body: Vec<u8>) -> Answer {
        Answer {
            head: format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n",
                body.len()
            ),
            body,
            short: 0,
        }
    }

    fn status(line: &str) -> Answer {
        Answer {
            head: format!("HTTP/1.1 {line}\r\nContent-Length: 0\r\n"),
            body: Vec::new(),
            short: 0,
        }
    }

    /// A stand-in for GitHub on the loopback: `route` answers each request
    /// from its method and path. Counts the GETs, which are downloads.
    async fn github(
        route: impl Fn(&str, &str) -> Answer + Send + Sync + 'static,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let gets = Arc::new(AtomicUsize::new(0));
        let counted = gets.clone();
        let route = Arc::new(route);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let route = route.clone();
                let counted = counted.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let request = String::from_utf8_lossy(&request).to_string();
                    let mut first = request.split_whitespace();
                    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
                    if method == "GET" {
                        counted.fetch_add(1, Ordering::SeqCst);
                    }
                    let answer = route(method, path);
                    let mut out = format!("{}Connection: close\r\n\r\n", answer.head).into_bytes();
                    if method != "HEAD" {
                        out.extend_from_slice(&answer.body[..answer.body.len() - answer.short]);
                    }
                    let _ = stream.write_all(&out).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (format!("http://127.0.0.1:{port}"), gets)
    }

    /// GitHub with `tag` as the latest release, served whole.
    async fn releases(tag: &'static str) -> (String, Arc<AtomicUsize>) {
        github(move |_, path| match path {
            "/latest/download/SideInstaller.ipa" => {
                redirect(&format!("/download/{tag}/SideInstaller.ipa"))
            }
            p if p == format!("/download/{tag}/SideInstaller.ipa") => file(ipa_bytes(tag)),
            _ => status("404 Not Found"),
        })
        .await
    }

    fn latest(base: &str) -> String {
        format!("{base}/latest/download/SideInstaller.ipa")
    }

    /// A port nothing listens on: GitHub, blocked.
    async fn blocked() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("http://127.0.0.1:{port}/latest/download/SideInstaller.ipa")
    }

    #[test]
    fn the_tag_is_read_from_the_release_download() {
        let tag = |url: &str| tag_of(url);
        assert_eq!(
            tag("https://github.com/FrizzleM/SideInstaller/releases/download/v2.1.1/SideInstaller.ipa"),
            Some("v2.1.1".into())
        );
        assert_eq!(
            tag("https://github.com/o/r/releases/download/v2.0.0-beta.1/SideInstaller.ipa?x=1#y"),
            Some("v2.0.0-beta.1".into())
        );
        assert_eq!(tag("https://github.com/o/r/releases/latest"), None);
        assert_eq!(
            tag("https://github.com/o/r/releases/download//SideInstaller.ipa"),
            None
        );
        assert_eq!(
            tag("https://github.com/login?return_to=/download/v1/SideInstaller.ipa"),
            None
        );
        assert_eq!(tag("SideInstaller.ipa"), None);
        assert_eq!(version("v2.1.1"), "2.1.1");
        assert_eq!(version("2.1.1"), "2.1.1");
    }

    #[tokio::test]
    async fn the_latest_release_is_downloaded_once_and_kept() {
        let home = home("kept");
        let (base, gets) = releases("v2.1.1").await;

        let ipa = get_from(None, &home, Mode::Get, &latest(&base))
            .await
            .unwrap();
        assert_eq!(ipa.release.as_deref(), Some("v2.1.1"));
        assert!(ipa.github);
        assert_eq!(ipa.name(), "SideInstaller 2.1.1");
        assert_eq!(std::fs::read(&ipa.path).unwrap(), ipa_bytes("v2.1.1"));
        let dir = home.join("downloads");
        let kept = kept(&dir).unwrap();
        assert_eq!(kept.release, "v2.1.1");
        assert_eq!(kept.size, ipa_bytes("v2.1.1").len() as u64);
        assert_eq!(
            kept.sha256,
            hex::encode(Sha256::digest(ipa_bytes("v2.1.1")))
        );
        assert!(!dir.join("SideInstaller.ipa.part").exists());
        assert_eq!(gets.load(Ordering::SeqCst), 1);

        // Still the latest: asked about, not downloaded again.
        let again = get_from(None, &home, Mode::Get, &latest(&base))
            .await
            .unwrap();
        assert_eq!(again.release.as_deref(), Some("v2.1.1"));
        assert!(again.github);
        assert_eq!(gets.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_newer_release_replaces_the_kept_one() {
        let home = home("newer");
        let (old, _) = releases("v2.1.0").await;
        get_from(None, &home, Mode::Get, &latest(&old))
            .await
            .unwrap();
        let (new, gets) = releases("v2.1.1").await;
        let ipa = get_from(None, &home, Mode::Get, &latest(&new))
            .await
            .unwrap();
        assert_eq!(ipa.release.as_deref(), Some("v2.1.1"));
        assert_eq!(std::fs::read(&ipa.path).unwrap(), ipa_bytes("v2.1.1"));
        assert_eq!(kept(&home.join("downloads")).unwrap().release, "v2.1.1");
        assert_eq!(gets.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn blocked_github_installs_the_kept_release() {
        let home = home("blocked");
        let (base, _) = releases("v2.1.1").await;
        get_from(None, &home, Mode::Get, &latest(&base))
            .await
            .unwrap();

        let ipa = get_from(None, &home, Mode::Get, &blocked().await)
            .await
            .unwrap();
        assert_eq!(ipa.release.as_deref(), Some("v2.1.1"));
        assert!(!ipa.github);
        assert_eq!(std::fs::read(&ipa.path).unwrap(), ipa_bytes("v2.1.1"));
        // `siboot check` says the same.
        let looked = get_from(None, &home, Mode::Look, &blocked().await)
            .await
            .unwrap();
        assert_eq!(looked.release.as_deref(), Some("v2.1.1"));
    }

    #[tokio::test]
    async fn blocked_github_with_nothing_kept_says_how_to_get_it() {
        let home = home("nothing");
        let fail = get_from(None, &home, Mode::Get, &blocked().await)
            .await
            .unwrap_err();
        assert!(
            fail.advice.starts_with("Couldn't reach GitHub"),
            "{}",
            fail.advice
        );
        assert!(fail.advice.contains("turn on a VPN that reaches it"));
        assert!(fail.advice.contains("--ipa /var/minis/mounts/"));
        assert!(!fail.raw.is_empty());
        let fail = get_from(None, &home, Mode::Look, &blocked().await)
            .await
            .unwrap_err();
        assert!(fail.advice.starts_with("Couldn't reach GitHub"));
    }

    #[tokio::test]
    async fn a_failed_download_falls_back_to_the_kept_release() {
        let home = home("fallback");
        let (old, _) = releases("v2.1.0").await;
        get_from(None, &home, Mode::Get, &latest(&old))
            .await
            .unwrap();

        // GitHub names a newer release, but its download is blocked.
        let (base, gets) = github(|_, path| match path {
            "/latest/download/SideInstaller.ipa" => redirect("/download/v2.1.1/SideInstaller.ipa"),
            _ => status("403 Forbidden"),
        })
        .await;
        let ipa = get_from(None, &home, Mode::Get, &latest(&base))
            .await
            .unwrap();
        assert_eq!(gets.load(Ordering::SeqCst), 1);
        assert_eq!(ipa.release.as_deref(), Some("v2.1.0"));
        assert!(ipa.github);
        assert_eq!(std::fs::read(&ipa.path).unwrap(), ipa_bytes("v2.1.0"));
        assert_eq!(kept(&home.join("downloads")).unwrap().release, "v2.1.0");

        let fresh = self::home("fallback-none");
        let fail = get_from(None, &fresh, Mode::Get, &latest(&base))
            .await
            .unwrap_err();
        assert!(
            fail.advice
                .starts_with("Couldn't download SideInstaller 2.1.1 from GitHub."),
            "{}",
            fail.advice
        );
        assert!(fail.raw.contains("403"), "{}", fail.raw);
    }

    #[tokio::test]
    async fn a_cut_off_or_wrong_download_is_never_kept() {
        let home = home("broken");
        let (old, _) = releases("v2.1.0").await;
        get_from(None, &home, Mode::Get, &latest(&old))
            .await
            .unwrap();
        let dir = home.join("downloads");

        for answer in ["short", "page"] {
            let (base, _) = github(move |_, path| match path {
                "/latest/download/SideInstaller.ipa" => {
                    redirect("/download/v2.1.1/SideInstaller.ipa")
                }
                _ if answer == "short" => Answer {
                    short: 1000,
                    ..file(ipa_bytes("v2.1.1"))
                },
                _ => file(b"<!DOCTYPE html><title>Rate limited</title>".to_vec()),
            })
            .await;
            let ipa = get_from(None, &home, Mode::Get, &latest(&base))
                .await
                .unwrap();
            assert_eq!(ipa.release.as_deref(), Some("v2.1.0"), "{answer}");
            assert_eq!(
                std::fs::read(&ipa.path).unwrap(),
                ipa_bytes("v2.1.0"),
                "{answer}"
            );
            assert!(!dir.join("SideInstaller.ipa.part").exists(), "{answer}");
        }
    }

    #[tokio::test]
    async fn check_downloads_nothing() {
        let home = home("look");
        let (base, gets) = releases("v2.1.1").await;
        let ipa = get_from(None, &home, Mode::Look, &latest(&base))
            .await
            .unwrap();
        assert_eq!(ipa.release.as_deref(), Some("v2.1.1"));
        assert!(ipa.github);
        assert_eq!(gets.load(Ordering::SeqCst), 0);
        assert!(kept(&home.join("downloads")).is_none());

        let (gone, _) = github(|_, path| match path {
            "/latest/download/SideInstaller.ipa" => redirect("/download/v2.1.1/SideInstaller.ipa"),
            _ => status("404 Not Found"),
        })
        .await;
        let fail = get_from(None, &home, Mode::Look, &latest(&gone))
            .await
            .unwrap_err();
        assert!(fail.raw.contains("404"), "{}", fail.raw);
    }

    #[tokio::test]
    async fn an_answer_that_names_no_release_counts_as_github_not_answering() {
        let home = home("portal");
        // A captive portal, or a proxy's own page, instead of GitHub.
        let (portal, _) = github(|_, _| redirect("http://login.example/portal")).await;
        let fail = get_from(None, &home, Mode::Get, &latest(&portal))
            .await
            .unwrap_err();
        assert!(fail.advice.starts_with("Couldn't reach GitHub"));
        assert!(fail.raw.contains("no release tag"), "{}", fail.raw);
        let (page, _) = github(|_, _| file(b"<html>".to_vec())).await;
        let fail = get_from(None, &home, Mode::Get, &latest(&page))
            .await
            .unwrap_err();
        assert!(fail.raw.contains("not a redirect"), "{}", fail.raw);
    }

    #[tokio::test]
    async fn an_ipa_by_url_is_downloaded_apart_from_the_kept_release() {
        let home = home("url");
        let (base, _) = releases("v2.1.1").await;
        get_from(None, &home, Mode::Get, &latest(&base))
            .await
            .unwrap();
        let (other, _) = github(|_, _| file(ipa_bytes("custom"))).await;
        let url = format!("{other}/Custom.ipa");

        let ipa = get_from(Some(&url), &home, Mode::Get, &blocked().await)
            .await
            .unwrap();
        assert_eq!(ipa.release, None);
        assert!(!ipa.github);
        assert!(ipa.path.ends_with("downloads/other.ipa"));
        assert_eq!(std::fs::read(&ipa.path).unwrap(), ipa_bytes("custom"));
        let dir = home.join("downloads");
        assert_eq!(kept(&dir).unwrap().release, "v2.1.1");
        assert_eq!(
            std::fs::read(dir.join("SideInstaller.ipa")).unwrap(),
            ipa_bytes("v2.1.1")
        );

        let fail = get_from(Some(&blocked().await), &home, Mode::Get, &latest(&base))
            .await
            .unwrap_err();
        assert!(fail
            .advice
            .starts_with("Couldn't download the IPA from 127.0.0.1"));
    }

    #[tokio::test]
    async fn an_ipa_by_path_is_used_as_it_is() {
        let home = home("path");
        let own = home.join("Mine.ipa");
        std::fs::write(&own, ipa_bytes("mine")).unwrap();
        let ipa = get_from(
            Some(own.to_str().unwrap()),
            &home,
            Mode::Get,
            &blocked().await,
        )
        .await
        .unwrap();
        assert_eq!(ipa.path, own);
        assert_eq!(ipa.name(), "your IPA");
        assert!(!ipa.github);
        let missing = home.join("missing.ipa");
        assert!(get_from(
            Some(missing.to_str().unwrap()),
            &home,
            Mode::Get,
            &blocked().await
        )
        .await
        .is_err());
    }

    #[test]
    fn only_a_zip_passes_as_an_ipa() {
        let dir = home("zip");
        let good = dir.join("good.ipa");
        std::fs::write(&good, b"PK\x03\x04rest").unwrap();
        assert!(check_ipa(&good).is_ok());
        let page = dir.join("page.ipa");
        std::fs::write(&page, b"<!DOCTYPE html>").unwrap();
        assert!(check_ipa(&page)
            .unwrap_err()
            .advice
            .contains("isn't an IPA"));
        let tiny = dir.join("tiny.ipa");
        std::fs::write(&tiny, b"PK").unwrap();
        assert!(check_ipa(&tiny).is_err());
        assert!(check_ipa(&dir.join("missing.ipa")).is_err());
    }

    #[test]
    fn a_kept_release_must_still_be_what_it_says() {
        let dir = home("described");
        assert_eq!(kept(&dir), None);
        let bytes = ipa_bytes("v2.1.1");
        std::fs::write(dir.join(KEPT_IPA), &bytes).unwrap();
        assert_eq!(kept(&dir), None, "no description");
        let described = Kept {
            release: "v2.1.1".into(),
            size: bytes.len() as u64,
            sha256: "ab".into(),
        };
        keep(&dir, &described).unwrap();
        assert_eq!(kept(&dir), Some(described.clone()));
        assert!(!dir.join("SideInstaller.json.part").exists());

        std::fs::write(dir.join(KEPT_IPA), &bytes[..100]).unwrap();
        assert_eq!(kept(&dir), None, "another size");
        let mut page = b"<html>".to_vec();
        page.resize(bytes.len(), b' ');
        std::fs::write(dir.join(KEPT_IPA), &page).unwrap();
        assert_eq!(kept(&dir), None, "not an IPA");
        std::fs::write(dir.join(KEPT_IPA), &bytes).unwrap();
        std::fs::write(dir.join(KEPT_INFO), "{\"release\":").unwrap();
        assert_eq!(kept(&dir), None, "an unreadable description");
    }
}
