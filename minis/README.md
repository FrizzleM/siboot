# ssboot for Open Minis

Installs SideStore onto the iPhone it runs on, signed with your own Apple
Account, from the Linux shell of [Open Minis](https://apps.apple.com/app/id6759188481)
(free, App Store): no computer, no certificate. It's siboot (this repository's
`main` branch, which installs SideInstaller) with SideStore in its place,
installed and handed its pairing the way SideInstaller installs SideStore. The
pairing comes from ssboot-pair in rootshell ([`../rootshell`](../rootshell)),
which runs first.

## Running it

Needs iOS 27 or later, Developer Mode on, Wi-Fi, LocalDevVPN (App Store)
connected, and ssboot-pair in rootshell paired and still running
([`../rootshell`](../rootshell#running-it)). Then, in Open Minis's terminal:

```sh
wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/ssboot/minis/install.sh | sh
```

ssboot opens a page over the terminal, in Minis's own browser
([below](#the-page)), that shows each step and asks its questions. The first
time, enter the transfer code rootshell shows; after that ssboot uses the
pairing file it saved and doesn't ask. If siboot has run in this Minis, ssboot
takes the pairing file siboot saved instead, so rootshell isn't needed, and
starts from siboot's signing certificate and sign-in. Then your Apple Account
email, password and verification code. Keep Minis open until ssboot says it's
done: iOS pauses it in the background.

Then open SideStore (the page's last box has a button for it). The first time,
trust your Apple Account in Settings › General › VPN & Device Management.
SideStore already has this iPhone's pairing: sign in to it with the same Apple
Account, and keep LocalDevVPN connected while it installs or refreshes apps.
Free accounts' apps last 7 days; refresh them in SideStore before then.

To answer in the terminal instead, add `-s -- --terminal` after `sh` (or run
`ssboot --terminal` once it's installed).

[`install.sh`](install.sh) downloads [`bin/ssboot-aarch64`](bin) from this
branch (a progress bar on one line while it does), checks it against the
SHA-256 the script carries, keeps it at `~/.ssboot/bin/ssboot` (and
`/usr/local/bin/ssboot`), and runs it. After that, `ssboot` on its own runs it
again without downloading.

## Which SideStore

ssboot installs SideStore's nightly
(`releases/download/nightly/SideStore.ipa`), as SideInstaller's Nightly setting
does. On iOS 27 that's the build that can use this iPhone's pairing: an
RPPairing record, which SideStore reads from its nightly of 20 September 2026
on. iOS 27 makes no classic lockdown pairing, and lets no app use one through
LocalDevVPN, while SideStore's newest release (`releases/latest`, 0.7.0-alpha)
is a build from 2024 that reads only that kind.

`--ipa PATH|URL` installs another SideStore IPA instead. Before signing, which
spends one of the week's App IDs, ssboot reads the IPA: it refuses one that
isn't SideStore as published (`com.SideStore.SideStore`; not LiveContainer +
SideStore), and warns about a build that doesn't read the nightlies' pairing
files, since it may not connect.

## What SideStore is handed

Signing does for SideStore what AltServer does, as isideload knows it by its
bundle ID: the app group goes into its Info.plist and its widget's
(`ALTAppGroups`), and the signing certificate into the app
(`ALTCertificate.p12`, named by `ALTCertificateID`). It's signed with an app
group and without Increased Memory Limit, which SideStore doesn't keep when it
renews its App ID.

After the install, house_arrest opens SideStore's container, and ssboot writes
what SideInstaller writes once it has installed SideStore
(`Engine.performWritePairing`, `handOffSplitPairing` and
`handOffSideStoreAnisette`), each file read back and checked:

| What | Where, in SideStore's container | For |
|---|---|---|
| The pairing file, whole, as XML | `Documents/ALTPairingFile.mobiledevicepairing` | Builds before the nightly of 20 September 2026 |
| The RPPairing record on its own | `Documents/PairingFile_RemoteRP.plist` | The nightlies since, which keep one file per protocol and turn down a file holding both |
| The lockdown record on its own, when the file has a complete one (never one from rootshell) | `Documents/PairingFile_Lockdown.plist` | The same |
| `isPairingReset` off, `activePairingProtocol` `rppairing` (or `lockdown`) | `Library/Preferences/<bundle ID>.plist` | The nightlies load neither file until their own import has set these |
| On-device anisette and auto-rotation off, the server "SideStore (New)" unless a macOS-ADI one is picked, client info saved for Windows or Android back to a Mac | The same file | SideStore's own sign-in, the way its users got past Apple's -80046 |
| The signing certificate, as SideStore imports it | `Documents/Account.sideconf` | Only a build that takes it without asking for a password. The nightly asks (it has `acctFileChecksum`), so it gets none, and may offer to resign itself the first time you sign in |

The settings file is written beside the old one and renamed over it, as
cfprefsd saves it. It's written after the install: installing is what makes iOS
read it again.

## The screen

With `--terminal`, ssboot keeps a small live area under what it has printed:
the steps of the run, the running one with a spinner, what it's doing and for
how long, and a progress bar for the download, the upload and the install. A
finished step stays as one line with what came of it, and a warning stays where
it happened, so scrolling back reads as a log of the run. It ends in a box: what
to do next, or why it stopped.

Under all of it, dim, is **A project by Frizzle**: from the first screen, under
every step and question, and still the last line once ssboot exits. On a screen
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
in place. With `--plain` or `--verbose`, `TERM=dumb`, `SSBOOT_PLAIN` set, or
without a terminal on both ends, ssboot prints plain lines instead, one per
event. `cargo run --example screens` plays the screen through a made-up run, on
a Mac and without an iPhone (`-- stop`, `-- developer-mode` and `-- --plain` for
the others).

## The page

In Open Minis ssboot shows the run as a page in Minis's own browser, to tap
rather than type (since 0.2.0; `--terminal` keeps it in the terminal, and
`--web` serves the page even where there's no Minis). ssboot serves the page
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
`minis-open`, ssboot prints the page's address instead. A page opened again
draws the same screen: it gets every event since the start. ssboot sends one
every 5 s while it's quiet, so after 15 s with nothing the page says ssboot
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
cancelled sign-in), ssboot asks **Copy the log to the clipboard?** under the
box: `y` or return copies it, `n` doesn't. The log is what the screen showed,
as plain lines with the time since the start, the answers to its questions
(only that something was typed: never a password or a code), idevice's and
isideload's own lines at info and above (not debug: that can carry what Apple
and the anisette server send) and the underlying error `--verbose` would show.
The Apple Account's email is cut to its first letter and domain, and long runs
of hex digits (the UDID, anisette identities) to their ends.

It's copied with Open Minis's own `apple-clipboard set`, which reads it on stdin
(Minis's terminal ignores OSC 52), and saved to `~/.ssboot/ssboot.log` either
way. Without a terminal on both ends ssboot doesn't ask, and only says where the
file is.

## The steps

| # | Step | How |
|---|---|---|
| 1 | Check | GitHub (SideStore's nightly) and Apple's sign-in server answer; the iPhone answers RPPairing on `10.7.0.1:49152` through LocalDevVPN |
| 2 | Pairing | the file saved in `~/.ssboot` on an earlier run, while the iPhone accepts it; else the one siboot saved in `~/.siboot`, which is then copied; else the one ssboot-pair offers on `127.0.0.1:61207`, for its transfer code (20 s for each answer), pair-verified with the iPhone |
| 3 | Connect | pair-verify → `createListener` → TLS-PSK → CDTunnel → software TCP → RSD, as `tunnel_create_rppairing_multihost` does; lockdownd gives the UDID |
| 4 | Apple ID | rust-core's `apple_session.rs`, compiled unchanged: saved token first, then GrandSlam + 2FA, Mac-ADI anisette servers in SideInstaller's Auto order. The first time, from a copy of siboot's isideload store when there is one |
| 5 | Sign | SideStore's nightly, read first (SideStore? which pairing files does it read? does it take `Account.sideconf` quietly?), then signed with isideload's `sign_app` (device registered, app group, no Increased Memory Limit), as SideInstaller signs it |
| 6 | Install | AFC upload to `/PublicStaging`, every file checked, `installation_proxy` with `PackageType: Developer` |
| 7 | Hand over | house_arrest writes the pairing files and settings [above](#what-sidestore-is-handed) into SideStore's container |

`ssboot check` runs step 1 only, and `ssboot connect` steps 1–3.
`ssboot developer-mode` runs steps 1–3 and then asks for Developer Mode
([below](#developer-mode)).
`SSBOOT_APPLE_ID` and `SSBOOT_PASSWORD` skip the prompts. Everything is kept in
`~/.ssboot`: the pairing file (mode 600), isideload's store (signing key,
anisette identity, saved token) and the download. The password is never
written anywhere. Nothing is written into `~/.siboot`: ssboot only reads it.

## Developer Mode

```sh
ssboot developer-mode
```

Before ssboot is installed, the same is
`wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/ssboot/minis/install.sh | sh -s developer-mode`.

It works as SideInstaller's Settings › Developer Mode does. ssboot opens the
install's tunnel (steps 1–3, without the internet checks), and lockdownd says
whether Developer Mode is on (`DeveloperModeStatus` in
`com.apple.security.mac.amfi`). When it's on, nothing is sent. When it's off,
AMFI's RSD shim (`com.apple.amfi.lockdown.shim.remote`) is asked to show the
switch under Settings › Privacy & Security, where you turn it on: iOS restarts,
and once you unlock it, asks you to tap Turn On.

`ssboot developer-mode --turn-on` turns it on instead, once you type `yes`: the
iPhone restarts at once, and Minis stops with it. iOS does that only while the
iPhone has no passcode. With one, it refuses, and ssboot shows the switch
instead.

It needs what the install needs: a pairing the iPhone accepts, and a tunnel
that opens. So it can't bring the switch back on an iPhone that has never been
paired, since pairing happens under that same Developer Mode page.

## Building

ssboot compiles SideInstaller's vendored idevice, isideload and
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

To publish a new build, copy it to `bin/ssboot-aarch64` and put its
`sha256sum` in `install.sh`'s `SHA256`, and its size in bytes in `SIZE`, in the
same commit: the script refuses a binary that doesn't match (`SIZE` only draws
its progress bar).

## Tests

`cargo test` runs on any host (macOS included): the SideStore hand-off (the
records split out of rootshell's file and out of a merged one, the legacy file,
the pairing settings, the anisette settings from fresh, kept and replaced
ones), the IPA read before signing (SideStore or not, an extension's Info.plist
not taken for the app's, what a nightly and an older build read), the first run
starting from siboot's store (copied once, keys still private), the screen's
pieces (widths, cutting and wrapping, keys as the terminal sends them, code
boxes, the progress bar, boxes), the log's cleaning (emails, UDIDs, options
without their values), the hand-off from rootshell against a stand-in for
ssboot-pair's side (the right code, a mistyped one, three wrong ones, something
that isn't a pairing file, nothing offered, a paused rootshell), argument
parsing, `developer-mode` against stand-ins for lockdownd and AMFI, the bundle
walk, 2FA answers, server order, the sign-in errors that stop it, install
refusals, and rust-core's own `apple_session.rs` and `error_text.rs` tests.
The page's server is tested over real loopback connections: paths without the
key, another Host, POSTs that aren't JSON or come from another origin, oversized
requests, events replayed and resumed after a reconnect, one progress event a
percent, every kind of question (an answer for another question refused, an
empty one or a code that isn't six digits asked again, cancelling), Stop, and
closing. ssboot-pair's own tests, with a fake iPhone, are in
[`../rootshell`](../rootshell#tests).

`cargo test -- --ignored` also reads a real SideStore IPA, named by
`SSBOOT_SIDESTORE_IPA`, as step 5 does.

## What's been tested

- **0.1.0** is siboot 0.16.1 with SideStore in SideInstaller's place. Steps 1–4
  and 6 are siboot's, with siboot's saved pairing and signing identity as
  fallbacks; step 5 reads and signs SideStore's nightly; step 7 hands SideStore
  its pairing and settings as SideInstaller does. Tested on macOS and Alpine
  aarch64 (76 tests), through `examples/screens.rs` in a 46-column terminal (an
  install, a stop, `--plain`), with step 5's read of SideStore's nightly of 27
  October 2026 (0.7.0-20261027.1507: SideStore, reads the per-protocol pairing
  files, asks for a password before it takes `Account.sideconf`), and with the
  Alpine build's `ssboot check`, which reached GitHub and Apple. `install.sh`
  installed it piped into `sh`, skipped a current copy and refused a tampered
  one; with no terminal at all it now runs ssboot instead of stopping at its
  `/dev/tty` check, which siboot's still does. Not run on an iPhone yet: the
  writes into SideStore's container, SideStore connecting with them, and the
  copy of siboot's files are untested.

- **0.2.0** shows the run as the page above, as siboot 0.17.0 does: the steps
  are unchanged, `--terminal` gives the terminal as before, and the last box
  has a button that opens SideStore (`sidestore://`). The page is siboot's,
  which ran in Minis on an iPhone; ssboot's own build of it was tested on macOS
  and Alpine aarch64 only.

siboot's own history, up to 0.17.0, is in its README on the `main` branch.
