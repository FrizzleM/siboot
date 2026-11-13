# siboot for Open Minis

Installs SideInstaller onto the iPhone it runs on, signed with your own Apple
Account, from the Linux shell of [Open Minis](https://apps.apple.com/app/id6759188481)
(free, App Store): no computer, no certificate. The pairing comes from
siboot-pair in rootshell ([`../rootshell`](../rootshell)), which runs first.

## Running it

Needs iOS 27 or later, Developer Mode on, Wi-Fi, LocalDevVPN (App Store)
connected, and siboot-pair in rootshell paired and still running
([`../rootshell`](../rootshell#running-it)). Then, in Open Minis's terminal:

```sh
wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/main/minis/install.sh | sh
```

siboot opens a page over the terminal, in Minis's own browser
([below](#the-page)), that shows each step and asks its questions. The first
time, enter the transfer code rootshell shows; after that siboot uses the
pairing file it saved and doesn't ask. Then your Apple Account email, password
and verification code. Keep Minis open until siboot says it's done: iOS pauses
it in the background. The first time, trust your Apple Account in Settings ›
General › VPN & Device Management. Free accounts' apps last 7 days;
SideInstaller can renew itself from then on.

To answer in the terminal instead, add `-s -- --terminal` after `sh` (or run
`siboot --terminal` once it's installed).

[`install.sh`](install.sh) downloads [`bin/siboot-aarch64`](bin) from this
repository (a progress bar on one line while it does), checks it against the
SHA-256 the script carries, keeps it at `~/.siboot/bin/siboot` (and
`/usr/local/bin/siboot`), and runs it. After that, `siboot` on its own runs it
again without downloading.

## The screen

With `--terminal`, siboot keeps a small live area under what it has printed:
the steps of the run, the running one with a spinner, what it's doing and for
how long, and a progress bar for the download, the upload and the install. A
finished step stays as one line with what came of it, and a warning stays where
it happened, so scrolling back reads as a log of the run. It ends in a box: what
to do next, or why it stopped.

Under all of it, dim, is **A project by Frizzle**: from the first screen, under
every step and question, and still the last line once siboot exits. On a screen
too short for it and a blank line above it, it makes way for the run's own
lines. Plain output has it after the version.

Questions are asked in place, and read keys as they're typed: the email in a
field, the password as dots, the transfer and verification codes in boxes that
send themselves at the sixth digit, yes or no on `y` or `n` (or the arrows and
return), and menus picked with the arrows or a digit. At the verification code,
a letter shows the other ways to get one (a text, a call, the devices, again).
Esc steps back or stops; Ctrl-C stops.

Every line is cut or wrapped to the terminal's width (an iPhone upright gives
about 45 columns), and the live area kept shorter than the screen, so it redraws
in place. With `--plain` or `--verbose`, `TERM=dumb`, `SIBOOT_PLAIN` set, or
without a terminal on both ends, siboot prints plain lines instead, one per
event, as before. `cargo run --example screens` plays the screen through a
made-up run, on a Mac and without an iPhone (`-- stop`, `-- developer-mode` and
`-- --plain` for the others).

## The page

In Open Minis siboot shows the run as a page in Minis's own browser, to tap
rather than type (since 0.17.0; `--terminal` keeps it in the terminal, and
`--web` serves the page even where there's no Minis). siboot serves the page
itself, on this iPhone's loopback, and opens it with
Minis's `minis-open`, which shows it in a sheet over the terminal. Minis stays
in front, so iOS doesn't pause it. Safari wouldn't do: behind it, iOS pauses
Minis within seconds, and a paused app takes the connection and never answers.

The page shows what the live screen shows: the steps, the running one with what
it's doing, for how long and a progress bar, what worked (folded away once its
step is done), the warnings where they happened, and the box the run ends in,
which has a button that opens SideInstaller once it's installed. Show details
adds what `--verbose` adds. The questions are fields and buttons: the email
with the email keyboard, the password as dots, the codes in boxes that send
themselves at the sixth digit (marked as one-time codes, which iOS can offer to
fill from Messages), Other ways for Apple's code, and yes or no. Stop, tapped
twice, stops the run as Ctrl-C does.

The terminal under the sheet keeps plain lines, as with `--plain`, and return
there opens the page again after it's been swiped away. Where there's no
`minis-open`, siboot prints the page's address instead. A page opened again
draws the same screen: it gets every event since the start. siboot sends one
every 5 s while it's quiet, so after 15 s with nothing the page says siboot
isn't answering (iOS has paused Minis) and to go back to Minis.

How the page is kept to this iPhone, and to itself ([`src/web.rs`](src/web.rs)):

- It's served on 127.0.0.1 only, at a port the system picks, so nothing on the
  Wi-Fi reaches it.
- Every path starts with 128 random bits, new each run. Any app on the iPhone,
  and any website open in a browser on it, can reach a loopback port, but none
  can guess them.
- The Host header must name the loopback and the port (no DNS rebinding), and
  answers are POSTed as JSON, from the page's own origin when the browser names
  one, so a form on another site can't send one.
- The page loads nothing from anywhere else, can't be framed, and sends no
  referrer: its address carries the key.
- An answer goes to the question that asked for it and no further. It's never
  sent back to the page, the log only says that something was typed, and the
  address isn't in the log either.

`cargo run --example screens -- --web` plays the made-up run through the page
on a Mac, once a browser has it open (`-- --web --open` opens it).

## When it stops

When a run stops on a problem (not when you stop it: Ctrl-C, nothing typed, a
cancelled sign-in), siboot asks **Copy the log to the clipboard?** under the
box: `y` or return copies it, `n` doesn't. The log is what the screen showed,
as plain lines with the time since the start, the answers to its questions
(only that something was typed: never a password or a code), idevice's and
isideload's own lines at info and above (not debug: that can carry what Apple
and the anisette server send) and the underlying error `--verbose` would show.
The Apple Account's email is cut to its first letter and domain, and long runs
of hex digits (the UDID, anisette identities) to their ends.

It's copied with Open Minis's own `apple-clipboard set`, which reads it on stdin
(Minis's terminal ignores OSC 52), and saved to `~/.siboot/siboot.log` either
way. Without a terminal on both ends siboot doesn't ask, and only says where the
file is.

## The steps

| # | Step | How |
|---|---|---|
| 1 | Check | GitHub and Apple's sign-in server answer; the iPhone answers RPPairing on `10.7.0.1:49152` through LocalDevVPN |
| 2 | Pairing | the file saved in `~/.siboot` on an earlier run, while the iPhone accepts it; else the one siboot-pair offers on `127.0.0.1:61207`, for its transfer code (20 s for each answer), pair-verified with the iPhone |
| 3 | Connect | pair-verify → `createListener` → TLS-PSK → CDTunnel → software TCP → RSD, as `tunnel_create_rppairing_multihost` does; lockdownd gives the UDID |
| 4 | Apple ID | rust-core's `apple_session.rs`, compiled unchanged: saved token first, then GrandSlam + 2FA, Mac-ADI anisette servers in SideInstaller's Auto order |
| 5 | Sign | the latest `SideInstaller.ipa` release, signed with isideload's `sign_app` (device registered, app group, no Increased Memory Limit), as Side by Side does |
| 6 | Install | AFC upload to `/PublicStaging`, every file checked, `installation_proxy` with `PackageType: Developer` |
| 7 | Hand over | house_arrest writes the pairing file to `Library/Application Support/rp_pairing_file.plist` and marks it imported (`importedPairingFileName`), as Side by Side hands one to SideInstaller |

`siboot check` runs step 1 only, and `siboot connect` steps 1–3.
`siboot developer-mode` runs steps 1–3 and then asks for Developer Mode
([below](#developer-mode)).
`SIBOOT_APPLE_ID` and `SIBOOT_PASSWORD` skip the prompts. Everything is kept in
`~/.siboot`: the pairing file (mode 600), isideload's store (signing key,
anisette identity, saved token) and the download. The password is never
written anywhere.

## Developer Mode

```sh
siboot developer-mode
```

Before siboot is installed, the same is
`wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/main/minis/install.sh | sh -s developer-mode`.

It works as SideInstaller's Settings › Developer Mode does. siboot opens the
install's tunnel (steps 1–3, without the internet checks), and lockdownd says
whether Developer Mode is on (`DeveloperModeStatus` in
`com.apple.security.mac.amfi`). When it's on, nothing is sent. When it's off,
AMFI's RSD shim (`com.apple.amfi.lockdown.shim.remote`) is asked to show the
switch under Settings › Privacy & Security, where you turn it on: iOS restarts,
and once you unlock it, asks you to tap Turn On.

`siboot developer-mode --turn-on` turns it on instead, once you type `yes`: the
iPhone restarts at once, and Minis stops with it. iOS does that only while the
iPhone has no passcode. With one, it refuses, and siboot shows the switch
instead.

It needs what the install needs: a pairing the iPhone accepts, and a tunnel
that opens. So it can't bring the switch back on an iPhone that has never been
paired, since pairing happens under that same Developer Mode page. Whether
iOS 27 still answers on its pairing port with Developer Mode off hasn't been
tried.

## Building

siboot compiles SideInstaller's vendored idevice, isideload and
apple-codesign-quick, and rust-core's `apple_session.rs` and `error_text.rs`,
from the [`SideInstaller`](../SideInstaller) submodule, pinned to one commit, so
check it out first: `git clone --recursive`, or `git submodule update --init` in
a clone you already have.

Any aarch64 Linux with a musl toolchain builds it; on a Mac, a Lima Alpine VM
does (`limactl create --name=siboot template:alpine`, then
`apk add build-base cmake perl clang linux-headers rustup`):

```sh
RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" \
  cargo build --release --locked --target aarch64-unknown-linux-musl
```

A static non-PIE executable, like the `pair_host` Minis's own packages ship.
`--target` must be explicit, or the static flags reach the proc macros too.

To publish a new build, copy it to `bin/siboot-aarch64` and put its
`sha256sum` in `install.sh`'s `SHA256`, and its size in bytes in `SIZE`, in the
same commit: the script refuses a binary that doesn't match (`SIZE` only draws
its progress bar).

## Tests

`cargo test` runs on any host (macOS included): the screen's pieces (widths,
cutting and wrapping, keys as the terminal sends them, code boxes, the progress
bar, boxes), the log's cleaning (emails, UDIDs, options without their values),
the hand-off from rootshell
against a stand-in for siboot-pair's side (the right code, a mistyped one, three
wrong ones, something that isn't a pairing file, nothing offered), argument
parsing, `developer-mode` against stand-ins for lockdownd and AMFI (on already,
off, unreadable, a declined restart, the passcode refusal, a hang-up), the bundle walk, 2FA answers, server order, the sign-in errors that
stop it, install refusals, and rust-core's own `apple_session.rs` and
`error_text.rs` tests. A test also stands in for a rootshell that iOS has
paused: the connection goes through and nothing answers. The page's server is
tested over real loopback connections: paths without the key, another Host,
POSTs that aren't JSON or come from another origin, oversized requests, events
replayed and resumed after a reconnect, one progress event a percent, every
kind of question (an answer for another question refused, an empty one or a
code that isn't six digits asked again, cancelling), Stop, and closing. siboot-pair's own
tests, with a fake iPhone, are in
[`../rootshell`](../rootshell#tests).

## What's been tested

- **On an iPhone (iOS 27, November 2026):** this route end to end. siboot
  0.12.0 in Open Minis's App Store build took the pairing from siboot-pair 0.1.0
  in rootshell and installed SideInstaller.
- **0.13.0** is 0.12.0 without what that route doesn't use: pairing inside
  Minis (through `minis-bonjour`, which only Minis's TestFlight builds have),
  its silent-audio keep-alive and notifications, and the Files › Minis › shared
  import. Steps 3–7 are unchanged. Tested on macOS and Alpine aarch64 (29 tests;
  `install.sh` installed it, skipped a current copy and refused a tampered one).
  On the iPhone it got stuck with rootshell offering: it took rootshell's offer
  before the saved file, and rootshell showed no request.
- **0.13.1** goes back to 0.12.0's order, the saved file first, and stops
  waiting for rootshell's answer after 20 s. Tested the same way; not yet run on
  an iPhone. The hand-off itself (two apps on one iPhone's loopback) hasn't
  completed on an iPhone yet: the runs that worked there used a file moved over
  the LAN or saved by an earlier run.
- **0.14.0** adds `siboot developer-mode`; the install is unchanged. Tested on
  macOS and Alpine aarch64 (39 tests, the new ones against stand-ins for
  lockdownd and AMFI; `install.sh | sh -s developer-mode` ran it). Not run on
  an iPhone yet. SideInstaller's own helper, which sends the same requests, has
  only read the status and shown the switch on an iPhone where Developer Mode
  was already on.

- **0.15.0** is 0.14.0 with the live screen above and `--plain`; what each step
  does is unchanged. Tested on macOS and Alpine aarch64 (51 tests), and through
  `examples/screens.rs` in a 46-column terminal: an install with a mistyped
  transfer code and the menu of other ways to get Apple's code, a stop, Developer
  Mode's yes or no, and `--plain`; `install.sh`'s bar in Alpine. Not run on an
  iPhone yet, so not in Minis's own terminal.

- **0.16.0** is 0.15.0 with the log above, offered when a run stops; the steps
  are unchanged. Tested on macOS and Alpine aarch64 (57 tests), and through
  `examples/screens.rs -- stop` in a 46-column terminal, where the copy goes
  through `pbcopy`. Not run on an iPhone yet, so `apple-clipboard` hasn't been
  called from siboot: its use follows Open Minis's source
  (`src/ios/NativeOffloads/ClipboardOffload.m`).

- **0.16.1** is 0.16.0 with the credit under the live area
  ([above](#the-screen)); the steps are unchanged. Tested on macOS and Alpine
  aarch64 (58 tests), through `examples/screens.rs` in a 46-column terminal (an
  install, a stop, `--plain`, and 10 and 8 rows, where it makes way), and with
  the Alpine build's `siboot check` on a 46-column pty; `install.sh` installed
  it and skipped a current copy. Not run on an iPhone yet.

- **0.16.2** signs in with the Apple Account email in lowercase. Apple Account
  names aren't case-sensitive, but GrandSlam's SRP proof hashes the name as
  sent, and users who typed it with a capital were told the account or password
  was wrong. A signing key saved under the capitalized spelling is moved to the
  lowercase one, so it's kept. Tested on macOS and Alpine aarch64 (59 tests).
  Not signed in to Apple with a capitalized email yet.

- **0.17.0** shows the run as the page above by default; the steps are
  unchanged, and `--terminal` gives the terminal as before. Tested on macOS
  (71 tests), and through `examples/screens.rs -- --web` in a
  browser at an iPhone's size: an install with a mistyped transfer code, Other
  ways for Apple's code and a mistyped one; a stop at Connect and the question
  about the log; Developer Mode's question and the page's Stop; a page reloaded
  mid-run; and the run paused with SIGSTOP, where the page said siboot wasn't
  answering until it was resumed. `siboot --web check` on a Mac stopped at the
  iPhone, as it does there, and asked about the log on the page. A test build
  with `--web` ran in Minis on an iPhone: the sheet opened and the page worked.

Not tested yet: other iPhones and iOS versions, an Apple Account at its
certificate or App ID limit (siboot then asks which certificate to revoke), and
a second run after the 7-day profile expires.
