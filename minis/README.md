# altboot for Open Minis

Installs AltStore Classic onto the iPhone it runs on, signed with your own Apple
Account, from the Linux shell of [Open Minis](https://apps.apple.com/app/id6759188481)
(free, App Store): no computer, no certificate, no other app to pair with.

altboot is siboot 0.18.0 (this repository's `main` branch, which installs
SideInstaller) made to install AltStore the way SideInstaller installs it
([below](#altstore)). Everything before the download is siboot's: the page,
pairing this iPhone with itself in Minis 1.16
([below](#pairing-in-minis)), the tunnel, the sign-in and the install, so it
needs Open Minis 1.16 or later.

## Running it

Needs iOS 27 or later, Developer Mode on, Wi-Fi, LocalDevVPN (App Store)
connected, and Open Minis 1.16 or later. Then, in Open Minis's terminal:

```sh
wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/altboot/minis/install.sh | sh
```

altboot opens a page over the terminal, in Minis's own browser
([below](#the-page)), that shows each step and asks its questions. The first
time, it pairs this iPhone with itself: the page says what to do, and when you
tap **Start pairing**, open Settings › Privacy & Security › Developer Mode,
scroll to the bottom, tap **Pair with altboot**, and enter your passcode, then
the 6-digit code altboot shows (it also comes as a notification). A player
opens over Minis meanwhile: leave it open, it keeps Minis running while you're
in Settings. Go back to Minis once Settings says it's paired. After that altboot
uses the pairing file it saved and doesn't ask. If siboot or ssboot has run in
this Minis, altboot starts from the pairing and the sign-in it saved instead
([below](#starting-from-siboot-or-ssboot)). Then your Apple Account email,
password and verification code. Keep Minis open until altboot says it's done:
iOS pauses it in the background.

Then, as altboot's last box says:

1. Open AltStore. The first time, trust your Apple Account in Settings ›
   General › VPN & Device Management.
2. Sign in to AltStore with the same Apple Account.
3. To install and refresh apps without a computer, tap **Set up Remote
   AltServer** in AltStore's Settings. Its pairing file is already in place, so
   the setup skips pairing. Remote AltServer works through LocalDevVPN: keep it
   connected while AltStore installs or refreshes apps. Free accounts' apps last
   7 days, so refresh them before then.

In Open Minis 1.15 or older altboot stops at once and says to update Minis from
the App Store (the page has a button for it). Minis doesn't tell the shell its
version, so altboot goes by `minis-bonjour`, which came with 1.16. altboot still
takes rootshell's file ([`../rootshell`](../rootshell#running-it)) whenever
altboot-pair is offering one, for the transfer code it shows.

To answer in the terminal instead, add `-s -- --terminal` after `sh` (or run
`altboot --terminal` once it's installed).

[`install.sh`](install.sh) downloads [`bin/altboot-aarch64`](bin) from this
branch (a progress bar on one line while it does), checks it against the
SHA-256 the script carries, keeps it at `~/.altboot/bin/altboot` (and
`/usr/local/bin/altboot`), and runs it. After that, `altboot` on its own runs it
again without downloading.

## AltStore

Step 5 and step 7 are where altboot differs from siboot. They do what
SideInstaller 2.0 does when AltStore is picked on its Install tab
(`InstallSource.altStore` in its `ios-app/SideStoreDownloader.swift`,
`Engine.handOffAltStorePairing`), with the vendored isideload this repository's
SideInstaller submodule already pins:

- **The download.** AltStore's own source, `https://apps.altstore.io` (it
  redirects to `cdn.altstore.io/file/altstore/apps.json`), is the list AltServer
  installs AltStore from. altboot reads AltStore Classic's entry there
  (`com.rileytestut.AltStore`; the beta is listed under `.Beta`, which isideload
  doesn't set up as AltStore) and takes the first version listed, from 2.3 on,
  whose `minOSVersion` and `maxOSVersion` take in this iPhone's iOS, as AltServer
  takes it. 2.3 is the first with Remote AltServer; earlier versions need
  AltServer on a computer for everything. Only a download over HTTPS is taken,
  and it has to match the SHA-256 the source lists for it, or it's deleted and
  the run stops. A copy left by an earlier run that still matches isn't fetched
  again. On 2026-11-14 the source listed 2.3.3, 2.3.2, 2.3.1 and 2.3 for iOS 27,
  each with a SHA-256.
- **The check.** Before signing, which spends one of the week's App IDs, the
  IPA's own Info.plist has to name AltStore Classic: anything else is refused,
  the beta included. altboot also looks in AltStore's binary for
  `ALTPairingFile.plist`, the file 2.3.1 and later read (step 7), and says
  so when an `--ipa` it was given is a 2.3 that doesn't, or older than 2.3.
- **Signing.** isideload's `sign_app`, with this iPhone registered, an app group
  and no Increased Memory Limit. isideload knows AltStore by its bundle ID and
  sets it up as AltServer does: this iPhone's UDID as `ALTDeviceID` in its
  Info.plist (AltStore registers that device and signs apps for it), the app
  group as `ALTAppGroups` in its Info.plist and its widget's, and the signing
  certificate as `ALTCertificate.p12` (its password the certificate's machine
  ID) named by `ALTCertificateID`, so AltStore signs with the same certificate
  once you sign in to it. It also seals the RPPairing record into the app as
  `ALTPairingFile.dat`, AES-GCM under the machine ID, where AltStore 2.3 read
  it. altboot reads the signed Info.plist back and warns if `ALTDeviceID` isn't
  this iPhone's UDID.
- **The pairing.** After the install, house_arrest writes the RPPairing record
  on its own (`identifier`, `public_key`, `private_key`, `alt_irk`, plain XML, as
  idevice writes it) to `Documents/ALTPairingFile.plist` in AltStore's container,
  where AltServer has written it since altstoreio/AltStore 5811912 (2.3.1 and
  later), and reads it back. AltStore takes it into its Keychain when Remote
  AltServer is set up, which then skips pairing. AltStore only connects with an
  RPPairing record (its `OnDeviceClient` refuses one without `private_key`), so
  a file without one writes nothing, and AltStore pairs during that setup
  instead. Like SideInstaller, altboot treats this as optional: if it can't be
  written, AltStore is installed all the same, and the last box says to follow
  the setup's own pairing.

The last box has a button that opens AltStore with `altstore-classic://`, the
URL scheme AltStore Classic's Info.plist declares besides `altstore`.

## Starting from siboot or ssboot

siboot and ssboot keep their files in `~/.siboot` and `~/.ssboot` in the same
Minis. When altboot has none of its own yet, it starts from theirs, siboot's
first:

- **The pairing (step 2).** After its own saved file, altboot tries each of
  theirs that the iPhone still accepts, and keeps a copy. They pair the same
  way, so the iPhone takes theirs as well, and nothing is paired again.
- **The sign-in (step 4).** With no isideload store of its own, altboot copies
  theirs (the signing key and certificate, the anisette identity, the saved
  sign-in) and the anisette server that last worked. One certificate then signs
  SideInstaller, SideStore and AltStore alike, instead of another one taking a
  free account's last place, or altboot asking which certificate to revoke.

## The screen

With `--terminal`, altboot keeps a small live area under what it has printed:
the steps of the run, the running one with a spinner, what it's doing and for
how long, and a progress bar for the download, the upload and the install. A
finished step stays as one line with what came of it, and a warning stays where
it happened, so scrolling back reads as a log of the run. It ends in a box: what
to do next, or why it stopped.

Under all of it, dim, is **A project by Frizzle**: from the first screen, under
every step and question, and still the last line once altboot exits. On a screen
too short for it and a blank line above it, it makes way for the run's own
lines. Plain output has it after the version.

Questions are asked in place, and read keys as they're typed: the email in a
field, the password as dots, the transfer and verification codes in boxes that
send themselves at the sixth digit, yes or no on `y` or `n` (or the arrows and
return), and menus picked with the arrows or a digit. At the verification code,
a letter shows the other ways to get one (a text, a call, the devices, again).
Before pairing, what to do in Settings is printed, and **Start pairing?** is a
yes or no; the pairing code then shows in large digits, three rows tall.
Esc steps back or stops; Ctrl-C stops.

Every line is cut or wrapped to the terminal's width (an iPhone upright gives
about 45 columns), and the live area kept shorter than the screen, so it redraws
in place. With `--plain` or `--verbose`, `TERM=dumb`, `ALTBOOT_PLAIN` set, or
without a terminal on both ends, altboot prints plain lines instead, one per
event. `cargo run --example screens` plays the screen through a made-up run, on
a Mac and without an iPhone (`-- stop`, `-- developer-mode` and `-- --plain`
for the others).

## The page

In Open Minis altboot shows the run as a page in Minis's own browser, to tap
rather than type (`--terminal` keeps it in the terminal, and `--web` serves the
page even where there's no Minis). altboot serves the page itself, on this
iPhone's loopback, and opens it with Minis's `minis-open`, which shows it in a
sheet over the terminal. Minis stays in front, so iOS doesn't pause it. Safari
wouldn't do: behind it, iOS pauses Minis within seconds, and a paused app takes
the connection and never answers.

The page shows what the live screen shows: the steps, the running one with what
it's doing, for how long and a progress bar, what worked (folded away once its
step is done), the warnings where they happened, and the box the run ends in,
which has a button that opens AltStore once it's installed. Show details adds
what `--verbose` adds. The questions are fields and buttons: the email with the
email keyboard, the password as dots, the codes in boxes that send themselves
at the sixth digit (marked as one-time codes, which iOS can offer to fill from
Messages), Other ways for Apple's code, and yes or no. Pairing shows what to do
in Settings on a card with **Start pairing**, then the pairing code in large
digits on a card of its own while Settings waits for it. Stop, tapped twice,
stops the run as Ctrl-C does.

The terminal under the sheet keeps plain lines, as with `--plain`, and return
there opens the page again after it's been swiped away. Where there's no
`minis-open`, altboot prints the page's address instead. A page opened again
draws the same screen: it gets every event since the start. altboot sends one
every 5 s while it's quiet, so after 15 s with nothing the page says altboot
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
cancelled sign-in), altboot asks **Copy the log to the clipboard?** under the
box: `y` or return copies it, `n` doesn't. The log is what the screen showed,
as plain lines with the time since the start, the answers to its questions
(only that something was typed: never a password or a code), idevice's and
isideload's own lines at info and above (not debug: that can carry what Apple
and the anisette server send) and the underlying error `--verbose` would show.
The Apple Account's email is cut to its first letter and domain, and long runs
of hex digits (the UDID, anisette identities) to their ends.

It's copied with Open Minis's own `apple-clipboard set`, which reads it on stdin
(Minis's terminal ignores OSC 52), and saved to `~/.altboot/altboot.log` either
way. Without a terminal on both ends altboot doesn't ask, and only says where
the file is.

## The steps

| # | Step | How |
|---|---|---|
| 1 | Check | AltStore's source (apps.altstore.io) and Apple's sign-in server answer; the iPhone answers RPPairing on `10.7.0.1:49152` through LocalDevVPN |
| 2 | Pairing | the file saved in `~/.altboot` on an earlier run, while the iPhone accepts it; else siboot's, then ssboot's, while the iPhone accepts it (copied); else the one altboot-pair offers on `127.0.0.1:61207`, for its transfer code (20 s for each answer), when it's offering; else a new pairing, made here ([below](#pairing-in-minis)); then pair-verified with the iPhone |
| 3 | Connect | pair-verify → `createListener` → TLS-PSK → CDTunnel → software TCP → RSD, as `tunnel_create_rppairing_multihost` does; lockdownd gives the UDID and the iOS version |
| 4 | Apple ID | rust-core's `apple_session.rs`, compiled unchanged: saved token first, then GrandSlam + 2FA, Mac-ADI anisette servers in SideInstaller's Auto order; the first time, from siboot's or ssboot's store |
| 5 | Sign | the newest AltStore from 2.3 on that apps.altstore.io lists for this iOS, checked against its listed SHA-256, then signed with isideload's `sign_app` (device registered, app group, no Increased Memory Limit), which sets it up as AltServer does ([above](#altstore)) |
| 6 | Install | AFC upload to `/PublicStaging`, every file checked, `installation_proxy` with `PackageType: Developer` |
| 7 | Hand over | house_arrest writes the RPPairing record to `Documents/ALTPairingFile.plist` in AltStore's container, as AltServer does |

`altboot check` runs step 1 only, and `altboot connect` steps 1–3.
`altboot developer-mode` runs steps 1–3 and then asks for Developer Mode
([below](#developer-mode)). `--ipa PATH|URL` installs that IPA instead of the
source's, once it's checked to be AltStore Classic.
`ALTBOOT_APPLE_ID` and `ALTBOOT_PASSWORD` skip the prompts. Everything is kept
in `~/.altboot`: the pairing file (mode 600), isideload's store (signing key,
anisette identity, saved token) and the download. The password is never
written anywhere.

## Pairing in Minis

From iOS 27 an iPhone can pair *with itself*, but only one way round: a host
advertises `_remotepairing-pairable-host._tcp` over Bonjour, Settings › Privacy &
Security › Developer Mode lists it as "Pair with …", and the iPhone dials in and
drives pair-setup. Dialling the iPhone instead only pair-*verifies*: it answers
`allowsPairSetup: false`.

The shell can't advertise: iOS refuses its multicast (no multicast entitlement),
and an app may only publish the Bonjour types its Info.plist declares. Open
Minis 1.16 declares this one and publishes it for the shell with
`minis-bonjour advertise`, which forwards each connection to a port inside the
shell ([`src/bridge.rs`](src/bridge.rs)). So altboot listens on a loopback port,
has Minis advertise it as **altboot**, and answers with idevice's
`PairableHost`, the responder SideInstaller uses ([`src/pairing.rs`](src/pairing.rs)):

1. The page asks you to get ready, with what to do in Settings, and waits for
   **Start pairing**. A "Pair with altboot" left by an earlier run that was
   killed is withdrawn first.
2. Minis advertises the pairing host under the host's identifier, as the
   iPhone requires, with the seven TXT keys idevice gives. If iOS hasn't let
   Minis use the local network yet (it asks the first time), or Minis's own
   Settings › Network › Local Discovery is off, altboot says so and asks again.
3. A notification says where to find the entry, posted while Minis is still in
   front, so iOS can ask about notifications first if it hasn't.
4. Silence plays through Minis's player (below) while you're in Settings. If
   its sheet is closed or the silence paused while altboot waits, altboot starts
   it again as soon as Minis is in front (`apple-player status`, every 3 s).
5. When the iPhone asks for the code, it shows on the page in large digits and
   comes as a notification. A stray connection or a mistyped code leaves the
   entry up for another tap, three connections in all.
6. The pairing file goes to `~/.altboot` (mode 600), with the host's altIRK, so
   a later pairing keeps the same host identity. The entry is withdrawn.
7. A notification asks you back to Minis. The silence stops once Minis is in
   front again (`minis-bonjour list`), and altboot pair-verifies the new file
   with the iPhone through LocalDevVPN.

### Keeping Minis awake

The pairing happens in Settings, with Minis in the background, and iOS
suspends a background app within seconds. `minis-bonjour` withdraws its
advertisements while Minis is suspended, so the "Pair with …" entry would
vanish before it could be tapped. That's the likeliest reason Minis's own
`pair_via_bonjour.py` (OpenMinis/MinisSkills, `ios-dev`) has never got the entry
to show. SideInstaller and StikPair play silent audio through pairing for the
same reason.

Minis has background audio, and `apple-player play` plays a file with the
playback session category, so altboot writes 16 minutes of silence (8 kHz,
8-bit WAV, 7.7 MB) to `~/.altboot` and plays it until pairing ends and you're
back in Minis ([`src/keepalive.rs`](src/keepalive.rs)). The player opens as a
sheet over whatever Minis shows (the page, or the terminal), and closing it
stops the silence, so altboot says to leave it open, and closes it itself.

## Developer Mode

```sh
altboot developer-mode
```

Before altboot is installed, the same is
`wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/altboot/minis/install.sh | sh -s developer-mode`.

It works as SideInstaller's Settings › Developer Mode does, and as siboot's
does. altboot opens the install's tunnel (steps 1–3, without the internet
checks), and lockdownd says whether Developer Mode is on (`DeveloperModeStatus`
in `com.apple.security.mac.amfi`). When it's on, nothing is sent. When it's
off, AMFI's RSD shim (`com.apple.amfi.lockdown.shim.remote`) is asked to show
the switch under Settings › Privacy & Security, where you turn it on: iOS
restarts, and once you unlock it, asks you to tap Turn On.

`altboot developer-mode --turn-on` turns it on instead, once you type `yes`: the
iPhone restarts at once, and Minis stops with it. iOS does that only while the
iPhone has no passcode. With one, it refuses, and altboot shows the switch
instead.

It needs what the install needs: a pairing the iPhone accepts (made first, the
same way, if there isn't one), and a tunnel that opens. So it can't bring the
switch back on an iPhone that has never been paired, since pairing happens under
that same Developer Mode page.

## Building

altboot compiles SideInstaller's vendored idevice, isideload and
apple-codesign-quick, and rust-core's `apple_session.rs` and `error_text.rs`,
from the [`SideInstaller`](../SideInstaller) submodule, pinned to one commit, so
check it out first: `git clone --recursive --branch altboot`, or
`git submodule update --init` in a clone you already have. That commit's
isideload already sets AltStore up (`SpecialApp::AltStore`); SideInstaller's
main branch has the same isideload.

Any aarch64 Linux with a musl toolchain builds it; on a Mac, a Lima Alpine VM
does (`limactl create --name=siboot template:alpine`, then
`apk add build-base cmake perl clang linux-headers rustup`):

```sh
RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" \
  cargo build --release --locked --target aarch64-unknown-linux-musl
```

A static non-PIE executable, like the `pair_host` Minis's own packages ship.
`--target` must be explicit, or the static flags reach the proc macros too.

To publish a new build, copy it to `bin/altboot-aarch64` and put its
`sha256sum` in `install.sh`'s `SHA256`, and its size in bytes in `SIZE`, in the
same commit: the script refuses a binary that doesn't match (`SIZE` only draws
its progress bar).

## Tests

`cargo test` runs on any host (macOS included). Besides siboot's (the screen,
the log's cleaning, pairing in Minis against a fake iPhone
([`src/fake_iphone.rs`](src/fake_iphone.rs)) and stand-ins for Minis's
commands, the hand-off from rootshell, argument parsing, `developer-mode`
against stand-ins for lockdownd and AMFI, the bundle walk, 2FA answers, server
order, the sign-in errors that stop it, install refusals, the page's server,
and rust-core's own tests), altboot's cover:

- AltStore's source: the first version listed that runs on this iOS, from 2.3
  on and over HTTPS; the iOS range (`minOSVersion`, `maxOSVersion`, none for an
  iOS that can't be read); a listed SHA-256 lowercased, and one that isn't one
  dropped; a version that doesn't read left out on its own; a source without
  AltStore Classic, and one that isn't JSON.
- The download, from a server on the loopback: the listed file kept and
  recognized on a second run, another file refused with nothing of it kept and
  the earlier copy left alone.
- The IPA read: AltStore 2.3.3 and 2.3 told apart by what they read, 2.2.2 as
  older than Remote AltServer, and the beta, SideStore and an already-signed
  AltStore refused; the app's own Info.plist among its widget's and frameworks'.
- The signed app's Info.plist read back (`ALTDeviceID`).
- The record for AltStore: an RPPairing file as XML with its four keys, a merged
  file's RPPairing part only, and none from a file without `private_key`.
- Starting from siboot's or ssboot's store: ssboot's when siboot has none,
  siboot's first, a private key's mode kept, altboot's own store kept once it
  has one, an empty store not taken, never its own.

`cargo test -- --ignored` also reads a real AltStore IPA named by
`ALTBOOT_ALTSTORE_IPA`, reads the live source, and downloads AltStore from it
twice (the second run keeps the first one's file).

`cargo run --example screens` plays an AltStore install as the page and the
live screen show it (`-- rootshell` plays rootshell's hand-off instead).

## What's been tested

- **0.1.0** is siboot 0.18.0 with steps 5 and 7 for AltStore, the start from
  siboot's or ssboot's files, and the names (`altboot`, `~/.altboot`,
  "Pair with altboot"). Tested on macOS and Alpine aarch64 (111 tests each),
  and against the live source on 2026-11-14: it listed 2.3.3, 2.3.2, 2.3.1 and
  2.3 for iOS 27.1, step 5 downloaded 2.3.3 (38.7 MB), matched its listed
  SHA-256 and kept that file on a second run, and read the IPA as AltStore
  Classic 2.3.3 that reads `ALTPairingFile.plist` (and whose `AltStoreCore`
  names `ALTDeviceID`, `ALTCertificateID`, `ALTCertificate.p12` and
  `ALTAppGroups`). `examples/screens.rs -- --web` played an install through the
  page at an iPhone's size, to the AltStore box and its `altstore-classic://`
  button. In Alpine, `install.sh` installed the static build, skipped a current
  copy and refused a tampered one; the build ran `--help`, and `check` reached
  apps.altstore.io and Apple's sign-in server and stopped at the iPhone, as it
  does off one. Not run on an iPhone yet, so not signed with an Apple Account:
  signing AltStore, installing it, AltStore picking up the file and setting up
  Remote AltServer follow SideInstaller's code, whose own notes list them as
  not verified on a device either.
- **0.1.1** stops in an Open Minis older than 1.16 before anything else, with
  its own message (update Open Minis in the App Store) and, on the page, an
  Update Open Minis button that opens the App Store, as siboot 0.18.1 does.
  Before, an older Minis went on to step 2 and stopped there, pointing at
  rootshell, unless a saved pairing still worked. No log is offered after it.

Not tested yet: an iPhone at all, an Apple Account at its certificate or App ID
limit (altboot then asks which certificate to revoke), and a second run after
the 7-day profile expires.
