//! `ssboot` for Open Minis: installs SideStore onto the iPhone it runs on, with
//! no computer and no paid certificate. It's siboot (which installs
//! SideInstaller the same way) with SideStore in its place, installed and handed
//! its pairing as SideInstaller installs it.
//!
//! Open Minis (App Store) runs Alpine Linux on an ARM64 build of iSH, so this is
//! an ordinary static aarch64 Linux binary built on the same vendored idevice
//! and isideload as SideInstaller. The pairing comes from rootshell, another
//! App Store app: its ssboot-pair (../rootshell) pairs this iPhone with itself
//! and hands the file over on the iPhone's loopback (see README.md).
//!
//! The steps:
//! 1 check · 2 pairing · 3 connect · 4 Apple ID · 5 sign · 6 install · 7 hand over.
//! `ssboot developer-mode` runs 1–3, then asks for Developer Mode (devmode.rs).
//!
//! The Apple ID password goes only to Apple (as an SRP proof, via isideload) and
//! is never written to disk or logged. Like SideInstaller, the developer token
//! and signing key are kept, in ~/.ssboot/isideload, so a second run needn't
//! sign in again.
//!
//! In Open Minis the run shows as a page in Minis's browser, served on this
//! iPhone's loopback, and asks its questions there (web.rs); `--terminal`
//! keeps it in the terminal.

mod account;
mod devmode;
mod fail;
mod handoff;
mod install;
mod pairing;
mod preflight;
mod report;
mod sign;
mod tunnel;
mod ui;
mod web;

// rust-core's saved-session sign-in, compiled here unchanged so ssboot signs in
// exactly as SideInstaller does (anisette identity rules, -80046 skip, token
// reuse). `crate::error_text` is the path it expects.
#[allow(dead_code)]
#[path = "../../SideInstaller/rust-core/src/apple_session.rs"]
mod apple_session;
#[allow(dead_code)]
#[path = "../../SideInstaller/rust-core/src/error_text.rs"]
mod error_text;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;

use fail::{Fail, Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Steps in the full run. Plain output's numbering stays fixed for `check`
/// and `connect`, so it means the same thing in every install mode.
const STAGES: usize = 7;
/// `developer-mode`: the install's first three steps, then Developer Mode.
const DEVMODE_STAGES: usize = 4;
/// The steps' names on the live screen, in order.
const STEP_NAMES: [&str; STAGES] = ["Check", "Pairing", "Connect", "Apple ID", "Sign", "Install", "Handover"];
const DEVMODE_NAME: &str = "Dev Mode";

/// LocalDevVPN's address for this iPhone: traffic to it comes back to the
/// device's own services.
const DEFAULT_DEVICE_IP: Ipv4Addr = Ipv4Addr::new(10, 7, 0, 1);
/// The device's RPPairing listener (remoted), as SideInstaller dials it.
const DEFAULT_RP_PORT: u16 = 49152;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Every step.
    All,
    /// Step 1 only.
    Check,
    /// Steps 1-3: take the pairing and prove the tunnel works with it.
    Connect,
    /// Steps 1-3, then show Developer Mode in Settings (or turn it on).
    DeveloperMode,
}

pub struct Args {
    pub command: Command,
    /// With `developer-mode`: turn it on, not only show it.
    pub turn_on: bool,
    pub verbose: bool,
    /// Plain lines, not the live screen.
    pub plain: bool,
    /// The run as a page in Open Minis's browser (`--web`), or in the terminal
    /// (`--terminal`). Neither: the page where there's Minis to show it.
    pub web: Option<bool>,
    pub device: SocketAddr,
    /// ~/.ssboot: the pairing file, isideload's store, downloads.
    pub home: PathBuf,
    pub pairing_file: Option<PathBuf>,
    pub apple_id: Option<String>,
    pub ipa: Option<String>,
    pub anisette: Option<String>,
}

impl Args {
    pub fn pairing_path(&self) -> PathBuf {
        self.pairing_file
            .clone()
            .unwrap_or_else(|| self.home.join("rp_pairing_file.plist"))
    }
}

fn default_home() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| ".".into());
    home.join(".ssboot")
}

/// Where siboot, which installs SideInstaller the same way, keeps its files in
/// this Minis: a pairing and a signing identity ssboot can start from.
pub fn siboot_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".siboot"))
}

enum Parsed {
    Run(Args),
    Help,
    Version,
}

fn parse_args(argv: impl IntoIterator<Item = String>) -> std::result::Result<Parsed, String> {
    let mut args = Args {
        command: Command::All,
        turn_on: false,
        verbose: false,
        plain: false,
        web: None,
        device: SocketAddr::new(IpAddr::V4(DEFAULT_DEVICE_IP), DEFAULT_RP_PORT),
        home: default_home(),
        pairing_file: None,
        apple_id: None,
        ipa: None,
        anisette: None,
    };
    let mut argv = argv.into_iter();
    let mut command_seen = false;
    while let Some(arg) = argv.next() {
        let mut value = |flag: &str| argv.next().ok_or(format!("{flag} needs a value"));
        match arg.as_str() {
            "-h" | "--help" | "help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            "-v" | "--verbose" => args.verbose = true,
            "--plain" => args.plain = true,
            "--web" => args.web = Some(true),
            "--terminal" => args.web = Some(false),
            "--device-ip" => {
                let ip: IpAddr = value("--device-ip")?
                    .parse()
                    .map_err(|_| "--device-ip needs an IP address".to_string())?;
                args.device.set_ip(ip);
            }
            "--port" => {
                let port = value("--port")?
                    .parse()
                    .map_err(|_| "--port needs a port number".to_string())?;
                args.device.set_port(port);
            }
            "--home" => args.home = PathBuf::from(value("--home")?),
            "--pairing-file" => args.pairing_file = Some(PathBuf::from(value("--pairing-file")?)),
            "--apple-id" => args.apple_id = Some(value("--apple-id")?),
            "--ipa" => args.ipa = Some(value("--ipa")?),
            "--anisette" => args.anisette = Some(value("--anisette")?),
            "--turn-on" => args.turn_on = true,
            "check" | "connect" | "developer-mode" if !command_seen => {
                command_seen = true;
                args.command = match arg.as_str() {
                    "check" => Command::Check,
                    "connect" => Command::Connect,
                    _ => Command::DeveloperMode,
                };
            }
            other => return Err(format!("unrecognised argument `{other}`")),
        }
    }
    if args.turn_on && args.command != Command::DeveloperMode {
        return Err("--turn-on goes with `ssboot developer-mode`".into());
    }
    Ok(Parsed::Run(args))
}

/// Kept narrow: Minis's terminal is an iPhone's, about 45 columns upright.
fn usage() {
    use std::io::IsTerminal;
    let styled = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let head = |text: &str| if styled { format!("\x1b[1m{text}\x1b[0m") } else { text.to_string() };
    let flag = |text: &str| if styled { format!("\x1b[36m{text}\x1b[0m") } else { text.to_string() };
    println!(
        "\
{name} {VERSION}
Installs SideStore onto this iPhone, from
Open Minis, with your own Apple Account.

{usage}
  ssboot [command] [options]

  Pair in rootshell first (ssboot-pair), and
  leave it running. With no command, every
  step runs: check, take the pairing from
  rootshell, connect, sign in, sign
  SideStore, install it, and hand it the
  pairing.

{commands}
  {check}
      Only check that LocalDevVPN, this iPhone
      and the internet are ready.
  {connect}
      Check, take the pairing, and only open the
      tunnel with it.
  {devmode}
      Connect, and if Developer Mode is off, show
      its switch under Settings › Privacy &
      Security. Sends nothing when it's on. With
      --turn-on, turn it on instead, after
      asking: the iPhone restarts. iOS allows
      that only without a passcode; with one,
      ssboot shows the switch instead.

{options}
  {apple_id}
      Apple Account to sign with (else
      SSBOOT_APPLE_ID, else it asks). The
      password comes from SSBOOT_PASSWORD or a
      prompt that shows only dots.
  {ipa}
      Install this SideStore IPA instead of
      SideStore's nightly.
  {anisette}
      Sign in through this anisette server only.
  {turn_on}
      With developer-mode: turn it on, not only
      show it.
  {device_ip}
      LocalDevVPN's address for this iPhone
      (default {ip}).
  {port}
      The iPhone's RPPairing port (default {rp}).
  {pairing_file}
      Default ~/.ssboot/rp_pairing_file.plist.
  {home}
      Where ssboot keeps its files (default
      ~/.ssboot).
  {terminal}
      Answer here in the terminal, not on the
      page in Minis's browser.
  {plain}
      Plain lines instead of the live screen
      in the terminal.
  {web}
      The page even outside Minis: ssboot
      prints its address.
  {verbose}
      Show the underlying protocol and library
      errors (plain lines, too).
  {help}
      Show this message.

{needs}
  iOS 27 or later, Developer Mode on, Wi-Fi,
  LocalDevVPN (App Store) connected, and the
  pairing from ssboot-pair in rootshell (or
  the one siboot saved, if it ran here).",
        name = head("ssboot"),
        usage = head("USAGE"),
        commands = head("COMMANDS"),
        options = head("OPTIONS"),
        needs = head("NEEDS"),
        check = flag("check"),
        connect = flag("connect"),
        devmode = flag("developer-mode"),
        apple_id = flag("--apple-id EMAIL"),
        ipa = flag("--ipa PATH|URL"),
        anisette = flag("--anisette URL"),
        turn_on = flag("--turn-on"),
        device_ip = flag("--device-ip IP"),
        port = flag("--port N"),
        pairing_file = flag("--pairing-file PATH"),
        home = flag("--home DIR"),
        plain = flag("--plain"),
        web = flag("--web"),
        terminal = flag("--terminal"),
        verbose = flag("-v, --verbose"),
        help = flag("-h, --help"),
        ip = DEFAULT_DEVICE_IP,
        rp = DEFAULT_RP_PORT,
    );
}

fn main() -> ExitCode {
    let parsed = match parse_args(std::env::args().skip(1)) {
        Ok(p) => p,
        Err(problem) => {
            eprintln!("ssboot: {problem}");
            eprintln!("try `ssboot --help`");
            return ExitCode::from(2);
        }
    };
    let args = match parsed {
        Parsed::Help => {
            usage();
            return ExitCode::SUCCESS;
        }
        Parsed::Version => {
            println!("ssboot {VERSION}");
            return ExitCode::SUCCESS;
        }
        Parsed::Run(args) => args,
    };

    // Served before anything is shown, so the page gets every event.
    let page = if args.web.unwrap_or_else(web::available) {
        match web::start() {
            Ok(url) => Some(url),
            Err(e) => {
                eprintln!("ssboot: couldn't serve the page ({e}), so it runs here instead");
                None
            }
        }
    } else {
        None
    };
    ui::init(args.verbose, args.plain || page.is_some());
    init_logging(args.verbose);
    ui::header(
        VERSION,
        if args.command == Command::DeveloperMode {
            "Shows Developer Mode, or turns it on."
        } else {
            "Installs SideStore on this iPhone."
        },
    );
    if let Some(url) = &page {
        show_page(url);
    }

    // Two workers: the software TCP stack inside the tunnel keeps moving while
    // a foreground task waits on it, and threads are cheap next to emulation.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            ui::failure("Couldn't start the async runtime.", &e.to_string());
            ui::close();
            return ExitCode::FAILURE;
        }
    };

    if page.is_some() {
        // A question waits for the page with this thread blocked, so the
        // select below can't see Ctrl-C then. A task on a worker can.
        runtime.spawn(async {
            if tokio::signal::ctrl_c().await.is_ok() {
                ui::stop_now("Stopped with Ctrl-C.");
            }
        });
    }
    let outcome = runtime.block_on(async {
        tokio::select! {
            outcome = pipeline(&args) => outcome,
            _ = tokio::signal::ctrl_c() => Err(Fail::by_user("Stopped with Ctrl-C.")),
        }
    });
    let code = match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => {
            ui::failure(&f.advice, &f.raw);
            report::after_failure(&args, &f);
            ExitCode::FAILURE
        }
    };
    ui::close();
    code
}

/// Opens the page in Minis's browser, and opens it again whenever return is
/// pressed in the terminal, for a page swiped away. These lines are for the
/// terminal only, and kept out of the log: the address carries the key.
fn show_page(url: &str) {
    web::on_stop(|| ui::stop_now("Stopped from the page."));
    if web::show(url) {
        println!("      The page is open in Minis's browser. Closed it?");
        println!("      Press return here to open it again.");
        println!("      To type here instead, stop with Ctrl-C");
        println!("      and run ssboot --terminal.");
        let url = url.to_string();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::stdin().lock().lines() {
                if line.is_err() {
                    break;
                }
                web::show(&url);
            }
        });
    } else {
        println!("      Open this in a browser on this iPhone, and");
        println!("      keep this terminal open:");
        println!("      {url}");
    }
}

/// idevice's and isideload's own logs go to stderr: warnings by default, and
/// everything they say with `--verbose`. The run's log (`report.rs`) keeps
/// what they say at info and above either way: debug can carry what Apple
/// and the anisette server send.
///
/// hyper-util warns on every connection that the socket refused
/// `TCP_USER_TIMEOUT`, which iSH doesn't implement; the connection is fine, so
/// that warning only shows with `--verbose`.
fn init_logging(verbose: bool) {
    use tracing_subscriber::filter::{LevelFilter, Targets};
    use tracing_subscriber::prelude::*;

    let level = if verbose { LevelFilter::DEBUG } else { LevelFilter::WARN };
    let quiet = if verbose { LevelFilter::DEBUG } else { LevelFilter::ERROR };
    let filter = Targets::new().with_default(level).with_target("hyper_util", quiet);
    // On the live screen, above its live area, so a warning doesn't tear it.
    if ui::is_live() {
        let fmt = tracing_subscriber::fmt::layer()
            .with_writer(ui::LogLines)
            .with_ansi(false)
            .with_target(false)
            .without_time()
            .with_filter(filter);
        let _ = tracing_subscriber::registry().with(fmt).with(record_layer()).try_init();
    } else {
        let fmt = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(verbose)
            .without_time()
            .with_filter(filter);
        let _ = tracing_subscriber::registry().with(fmt).with(record_layer()).try_init();
    }
}

/// The libraries' lines at info and above, into the run's log.
fn record_layer<S>() -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    use tracing_subscriber::filter::{LevelFilter, Targets};
    use tracing_subscriber::Layer as _;

    let kept = Targets::new()
        .with_default(LevelFilter::INFO)
        .with_target("hyper_util", LevelFilter::ERROR);
    tracing_subscriber::fmt::layer()
        .with_writer(ui::RecordLines)
        .with_ansi(false)
        .with_target(true)
        .without_time()
        .with_filter(kept)
}

async fn pipeline(args: &Args) -> Result<()> {
    std::fs::create_dir_all(&args.home)
        .map_err(|e| Fail::new(format!("Couldn't create {}.", args.home.display()), e.to_string()))?;

    let devmode = args.command == Command::DeveloperMode;
    match args.command {
        Command::All => ui::plan(&STEP_NAMES, STAGES),
        Command::Check => ui::plan(&STEP_NAMES[..1], STAGES),
        Command::Connect => ui::plan(&STEP_NAMES[..3], STAGES),
        Command::DeveloperMode => {
            let mut names = STEP_NAMES[..3].to_vec();
            names.push(DEVMODE_NAME);
            ui::plan(&names, DEVMODE_STAGES);
        }
    }
    // Before the request, a failure also says what ssboot can't do without the
    // tunnel.
    let note = |f: Fail| if devmode { devmode::unreachable_note(f) } else { f };

    if devmode {
        ui::step(1, "Checking LocalDevVPN and this iPhone");
    } else {
        ui::step(1, "Checking LocalDevVPN, this iPhone and the internet");
    }
    let online = preflight::run(args).await.map_err(note)?;
    ui::done(if online { "online, and the iPhone answers" } else { "the iPhone answers" });
    if args.command == Command::Check {
        ui::finish("Ready", &["Run ssboot to install SideStore."]);
        return Ok(());
    }

    ui::step(2, "Taking the pairing from rootshell");
    let pairing_path = args.pairing_path();
    let source = pairing::ensure(args, &pairing_path).await.map_err(note)?;
    ui::done(match source {
        pairing::Source::Saved => "the saved file still works",
        pairing::Source::Siboot => "the file siboot saved works",
        pairing::Source::Rootshell => "taken from rootshell",
    });

    ui::step(3, "Connecting to this iPhone through LocalDevVPN");
    let mut connection = tunnel::open(args.device, &pairing_path).await.map_err(note)?;
    let device = connection.describe().await.map_err(note)?;
    ui::ok(&format!(
        "{} — {} on iOS {}",
        device.name, device.product_type, device.ios_version
    ));
    ui::detail(&format!("UDID {}", device.udid));
    ui::done(&format!("{} · iOS {}", device.name, device.ios_version));
    if args.command == Command::Connect {
        ui::finish("The tunnel works", &["This iPhone accepts the pairing file, and the tunnel opens with it."]);
        return Ok(());
    }
    if devmode {
        ui::step(4, "Asking this iPhone for Developer Mode");
        return developer_mode(args, &mut connection).await;
    }
    // The connection is closed here: iOS drops an idle tunnel during sign-in and
    // signing anyway, so a fresh one is opened to install (as Side by Side does).
    drop(connection);

    ui::step(4, "Signing in to your Apple Account");
    let mut session = account::sign_in(args).await?;
    ui::done(&format!("team {}", session.team));

    ui::step(5, "Signing SideStore for this iPhone");
    let ipa = sign::fetch_ipa(args).await?;
    let build = sign::inspect(&ipa)?;
    let version = build.short_version().map(str::to_string);
    ui::ok(&match &version {
        Some(version) => format!("it's SideStore {version}"),
        None => "it's SideStore".to_string(),
    });
    if !build.reads_split_pairing {
        ui::warn(
            "This SideStore build doesn't read the pairing files of SideStore's nightlies since 20 \
             September 2026. It may read only a lockdown pairing, which iOS 27 doesn't let an app \
             use on the iPhone itself, so it may not connect. Without --ipa, ssboot installs the \
             nightly, which reads this iPhone's pairing.",
        );
    }
    let signed = sign::sign(&mut session, &ipa, &device).await?;
    // Built now, while the tunnel is closed: iOS drops one left idle.
    let account_config = if build.imports_account_silently {
        ui::activity("preparing the signing certificate for SideStore");
        match account::account_config(&mut session).await {
            Ok(config) => Some(config),
            Err(e) => {
                ui::warn(
                    "Couldn't prepare the signing certificate for SideStore, so it may offer to \
                     resign itself the first time you sign in to it.",
                );
                ui::detail(&e);
                None
            }
        }
    } else {
        ui::detail(
            "this build asks for a password before it takes a certificate file, so it's given \
             none, as SideInstaller does",
        );
        None
    };
    ui::done(&match &version {
        Some(version) => format!("{} {version}", signed.name),
        None => format!("{} signed", signed.name),
    });

    ui::step(6, "Installing it");
    let mut connection = tunnel::open(args.device, &pairing_path).await?;
    let uploaded = install::install(&mut connection, &signed.app_dir).await?;
    ui::ok(&format!("{} installed ({})", signed.name, signed.bundle_id));
    ui::done(&format!("installed · {uploaded}"));

    ui::step(7, "Handing SideStore the pairing");
    let handed = handoff::hand_over(
        &mut connection,
        &signed.bundle_id,
        &pairing_path,
        &device,
        account_config.as_deref(),
    )
    .await?;
    ui::done("SideStore has the pairing");

    let mut next = vec![
        "Open SideStore from your Home Screen. The first time, iOS asks you to trust your Apple \
         Account: Settings › General › VPN & Device Management.",
        "It has this iPhone's pairing already. Sign in to it with the same Apple Account, and \
         keep LocalDevVPN connected while it installs or refreshes apps.",
    ];
    if !handed.certificate {
        next.push("If it offers to resign itself when you sign in, let it.");
    }
    next.push("Free accounts' apps last 7 days: refresh them in SideStore before then.");
    // Its own URL scheme (Info.plist): the page's button opens it.
    ui::link("Open SideStore", "sidestore://");
    ui::finish("SideStore is installed", &next);
    Ok(())
}

/// Step 4 of `ssboot developer-mode`, through the open tunnel.
async fn developer_mode(args: &Args, connection: &mut tunnel::Connection) -> Result<()> {
    let request = if args.turn_on { devmode::Request::TurnOn } else { devmode::Request::Show };
    let confirm = || ui::confirm("turn it on now?", false);
    let outcome = devmode::run(connection, request, confirm).await?;
    let unlock = "Once you unlock it, tap Turn On.";
    match outcome {
        devmode::Outcome::AlreadyOn => {
            ui::done("already on");
            ui::finish("Developer Mode is already on", &["So nothing was sent."]);
        }
        devmode::Outcome::Shown => {
            ui::done("the switch now shows in Settings");
            ui::finish(
                "Developer Mode is in Settings",
                &[
                    "It now shows under Settings › Privacy & Security, near the bottom. Turn it \
                     on there; iOS restarts to finish.",
                    unlock,
                ],
            );
        }
        devmode::Outcome::ShownForPasscode => {
            ui::done("the switch now shows in Settings");
            ui::finish(
                "Developer Mode is in Settings",
                &[
                    "iOS turns Developer Mode on from here only while the iPhone has no passcode, \
                     so ssboot showed it under Settings › Privacy & Security instead. Turn it on \
                     there; iOS restarts to finish.",
                    unlock,
                ],
            );
        }
        devmode::Outcome::TurningOn => {
            ui::done("set to turn on");
            ui::finish(
                "Developer Mode is turning on",
                &["This iPhone restarts now, and Minis stops with it.", unlock],
            );
        }
        devmode::Outcome::HungUp => {
            ui::done("the iPhone hung up, restarting");
            ui::finish(
                "This iPhone is restarting",
                &[
                    "It hung up right after the request, as it does when it restarts to turn \
                     Developer Mode on.",
                    unlock,
                    "If it doesn't restart, run ssboot developer-mode again.",
                ],
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Args {
        match parse_args(line.split_whitespace().map(String::from)).unwrap() {
            Parsed::Run(args) => args,
            _ => panic!("not a run"),
        }
    }

    #[test]
    fn defaults_run_everything_through_localdevvpn() {
        let args = parse("");
        assert_eq!(args.command, Command::All);
        assert_eq!(args.device.to_string(), "10.7.0.1:49152");
        assert!(args.pairing_path().ends_with(".ssboot/rp_pairing_file.plist"));
    }

    #[test]
    fn commands_and_options_parse() {
        let args = parse("connect --device-ip 192.168.1.20 --port 50000 -v");
        assert_eq!(args.command, Command::Connect);
        assert_eq!(args.device.to_string(), "192.168.1.20:50000");
        assert!(args.verbose);
        assert_eq!(parse("check").command, Command::Check);
        assert!(parse("--plain").plain);
        assert!(!parse("").plain);
        assert_eq!(parse("--web").web, Some(true));
        assert_eq!(parse("developer-mode --web").web, Some(true));
        assert_eq!(parse("--terminal").web, Some(false));
        assert_eq!(parse("--plain").web, None);
        assert_eq!(parse("").web, None);
    }

    #[test]
    fn developer_mode_parses_with_or_without_turn_on() {
        let args = parse("developer-mode");
        assert_eq!(args.command, Command::DeveloperMode);
        assert!(!args.turn_on);
        assert!(parse("developer-mode --turn-on").turn_on);
        assert!(parse("--turn-on developer-mode -v").turn_on);
    }

    #[test]
    fn bad_input_is_refused() {
        for line in [
            "--device-ip",
            "--device-ip nope",
            "pair",
            "check connect",
            "developer-mode check",
            "--turn-on",
            "connect --turn-on",
            "--repair",
            "--advertiser minis",
        ] {
            assert!(parse_args(line.split_whitespace().map(String::from)).is_err(), "{line}");
        }
    }
}
