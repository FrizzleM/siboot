# siboot for Open Minis

Installs SideInstaller onto the iPhone it runs on, signed with your own Apple
Account, from the Linux shell of [Open Minis](https://apps.apple.com/app/id6759188481)
(free, App Store): no computer, no certificate, no other app to pair with. Since
Open Minis 1.16 (App Store, 2026-11-14) siboot pairs the iPhone with itself too
([below](#pairing-in-minis)), so it needs 1.16 or later.

## Running it

Needs iOS 27 or later, Developer Mode on, Wi-Fi, LocalDevVPN (App Store)
connected, and Open Minis 1.16 or later. Then, in Open Minis's terminal:

```sh
wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/main/minis/install.sh | sh
```

siboot opens a page over the terminal, in Minis's own browser
([below](#the-page)), that shows each step and asks its questions. The first
time, it pairs this iPhone with itself: the page says what to do, and when you
tap **Start pairing**, open Settings › Privacy & Security › Developer Mode,
scroll to the bottom, tap **Pair with siboot**, and enter your passcode, then
the 6-digit code siboot shows (it also comes as a notification). A player opens
over Minis meanwhile: leave it open, it keeps Minis running while you're in
Settings. Go back to Minis once Settings says it's paired. After that siboot
uses the pairing file it saved and doesn't ask. Then your Apple Account email,
password and verification code, and, if the account is on more than one
developer team (a paid membership and the free team every account has, say),
which team signs ([below](#more-than-one-team)). The next time, siboot offers
the Apple Accounts that signed in before, and asks for no password or code
while Apple's sign-in lasts ([below](#saved-apple-accounts)). Keep Minis open until siboot
says it's done: iOS pauses it in the background. The first time, trust your
Apple Account in Settings › General › VPN & Device Management. Free accounts'
apps last 7 days; SideInstaller can renew itself from then on. A paid team's
last a year.

In Open Minis 1.15 or older siboot stops at once and says to update Minis from
the App Store (the page has a button for it). Minis doesn't tell the shell its
version, so siboot goes by `minis-bonjour`, which came with 1.16. siboot still
takes rootshell's file ([`../rootshell`](../rootshell#running-it)) whenever
siboot-pair is offering one, for the transfer code it shows.

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
Before pairing, what to do in Settings is printed, and **Start pairing?** is a
yes or no; the pairing code then shows in large digits, three rows tall.
Esc steps back or stops; Ctrl-C stops.

Every line is cut or wrapped to the terminal's width (an iPhone upright gives
about 45 columns), and the live area kept shorter than the screen, so it redraws
in place. With `--plain` or `--verbose`, `TERM=dumb`, `SIBOOT_PLAIN` set, or
without a terminal on both ends, siboot prints plain lines instead, one per
event, as before. `cargo run --example screens` plays the screen through a
made-up run, on a Mac and without an iPhone (`-- stop`, `-- developer-mode`,
`-- teams`, `-- moved`, `-- nowhere` and `-- --plain` for the others).

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
fill from Messages), Other ways for Apple's code, a button for each team when
there's a choice, and yes or no. Pairing shows
what to do in Settings on a card with **Start pairing**, then the pairing code
in large digits on a card of its own while Settings waits for it. Stop, tapped
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
| 1 | Check | GitHub and Apple's sign-in server answer; the iPhone answers RPPairing on `10.7.0.1:49152` through LocalDevVPN, or on the port its pairing service has instead ([below](#the-pairing-port)) |
| 2 | Pairing | the file saved in `~/.siboot` on an earlier run, while the iPhone accepts it; else the one siboot-pair offers on `127.0.0.1:61207`, for its transfer code (20 s for each answer), when it's offering; else a new pairing, made here ([below](#pairing-in-minis)); then pair-verified with the iPhone |
| 3 | Connect | pair-verify → `createListener` → TLS-PSK → CDTunnel → software TCP → RSD, as `tunnel_create_rppairing_multihost` does; lockdownd gives the UDID |
| 4 | Apple ID | the Apple Account picked from those saved, or typed, and its saved token before any password ([below](#saved-apple-accounts)); then rust-core's `apple_session.rs`, compiled unchanged: saved token first, then GrandSlam + 2FA, Mac-ADI anisette servers in SideInstaller's Auto order; then the team, picked when Apple lists more than one ([below](#more-than-one-team)) |
| 5 | Sign | the latest `SideInstaller.ipa` release, signed with isideload's `sign_app` (device registered, app group, no Increased Memory Limit), as Side by Side does |
| 6 | Install | AFC upload to `/PublicStaging`, every file checked, `installation_proxy` with `PackageType: Developer` |
| 7 | Hand over | house_arrest writes the pairing file to `Library/Application Support/rp_pairing_file.plist` and marks it imported (`importedPairingFileName`), as Side by Side hands one to SideInstaller |

`siboot check` runs step 1 only, and `siboot connect` steps 1–3.
`siboot developer-mode` runs steps 1–3 and then asks for Developer Mode
([below](#developer-mode)).
`SIBOOT_APPLE_ID` and `SIBOOT_PASSWORD` skip the prompts. Everything is kept in
`~/.siboot`: the pairing file (mode 600), isideload's store (signing key,
anisette identity, saved token), the saved Apple Accounts' emails
(`apple-accounts`, mode 600) and the download. The password is never written
anywhere.

## The pairing port

siboot reaches the iPhone's pairing service (remotepairingd's RPPairing
listener) at `10.7.0.1:49152` through LocalDevVPN, as SideInstaller and
StikDebug do. That port isn't fixed. The listener takes whatever port iOS hands
it when it starts, and iOS hands TCP ports out in order from 49152, not at
random (`net.inet.tcp.randomize_ports` is 0, as on a Mac). Started first after
a restart, it gets 49152; started after something else, a later one. On
2026-11-18 an iPhone on a Wi-Fi advertised its pairing service
(`_remotepairing._tcp`) on 55021, while users with Developer Mode on and
LocalDevVPN connected saw 0.19.0 stop with "The iPhone refused the connection
at 10.7.0.1:49152".

Since 0.19.1, when 49152 refuses, or what answers there isn't the pairing
service, siboot knocks on the iPhone's other dynamic ports, 49153 to 65535
without lockdownd's 62078, 32 at a time through LocalDevVPN, and takes the
first that answers the pairing handshake ([`src/preflight.rs`](src/preflight.rs)).
The later steps dial that port. A closed port refuses at once, so this takes
seconds: from a Mac over Wi-Fi, all 16,382 took 24 s. It stops early, with its
own advice, after 64 connections in a row that neither refuse nor connect
(LocalDevVPN has gone), when more than 48 ports take the connection (something
other than this iPhone answers there, like another VPN or a proxy), or when
Minis keeps running out of connections.

Minis's shell can't browse Bonjour itself. `minis-bonjour browse` and
`resolve` might give the advertised port, but whether `resolve` reaches the
iPhone's own service hasn't been tried, so siboot doesn't count on it.

## More than one team

Apple lists every developer team an Apple Account is on: the free one every
account has (Xcode's Personal Team), and a paid Apple Developer Program
membership too when there is one, the account's own or a company's. Until
0.19.0 siboot took the first. Now, when there's more than one, it asks which
signs ([`src/team.rs`](src/team.rs)), each marked **Free** or **Paid**: what a
free team signs lasts 7 days, what a paid team signs, a year.

- listTeams doesn't say free or paid outright, and isideload keeps only each
  team's name, ID and type, so siboot asks Apple for the list once more and
  reads the memberships as AltSign does (AltStore and SideStore go by it): a
  company's or organization's team is paid; an individual's is free when its
  one membership has "free" in its name, and paid otherwise. A team it can't
  tell is listed without either, and so is every team if that request fails.
  With one team there's no question and no extra request.
- The teams stay in Apple's order and the menu starts on the first, so return
  takes the team siboot took before. It's also the one SideInstaller signs
  with when it renews itself (its rust-core still takes the first), so signed
  by another team, SideInstaller can't renew itself in place: unless the team
  is paid, the box the run ends in says to run siboot again instead.
- The team's ID ends SideInstaller's bundle ID, so picking another team than
  last time installs a second copy beside the first.
- Back, Esc or an empty answer stops the run, and no log is offered.

## Saved Apple Accounts

Since 0.20.0 siboot remembers the Apple Accounts that sign in with it, so a
later run needn't ask for them ([`src/saved.rs`](src/saved.rs)):

- After a sign-in, the account's email goes first in `~/.siboot/apple-accounts`
  (one a line, mode 600, seven at most), and the first time, step 4 says so.
  The first run of 0.20.0 starts that list with the accounts that signed in
  with an earlier siboot, from the tokens they left (below), the latest first.
- The next run asks **Sign in with which Apple Account?**: each saved one, the
  last used first, then **Another Apple Account**, the email field as before
  (Cancel comes back to the menu), and **Forget an Apple Account**, which asks
  which one, then yes or no. Forgetting one takes its email off the list and
  deletes its saved token, but keeps its signing key, as SideInstaller does
  when it removes an Apple ID: signing in with it again makes no new
  certificate. Back, Esc or an empty answer at the menu stops the run.
- The password isn't saved: Minis's shell has no keychain to keep it in. What
  spares it is the developer token `apple_session.rs` saves after a sign-in, as
  SideInstaller does. `open` takes the password before it tries the token, so
  siboot tries the token first itself ([`src/account.rs`](src/account.rs)), the
  way `open` does but never logging in: on a GrandSlam client without the URL
  bag, then on one with it, through each anisette server in turn. When Apple
  takes it, step 4 says "signed in without the password", with no
  verification code either.
- When the token has expired (Apple's expiry, ten minutes early, as
  `apple_session.rs` reads it), or Apple turns it down, step 4 says the saved
  sign-in has run out and asks for the password, and the sign-in goes on as
  before. When no server gets the token to Apple, or Apple is limiting
  sign-ins, siboot stops as it would have with the password: `open` doesn't
  log in for those either.
- `--apple-id` or `SIBOOT_APPLE_ID` skips the menu, and the token is still
  tried first. `SIBOOT_PASSWORD` skips that try; `open` then tries the token
  itself.

How long Apple's token lasts hasn't been measured here: Apple states it in the
token, and Show details (or `--verbose`) says how long it has left.

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
shell ([`src/bridge.rs`](src/bridge.rs)). So siboot listens on a loopback port,
has Minis advertise it as **siboot**, and answers with idevice's
`PairableHost`, the responder SideInstaller uses ([`src/pairing.rs`](src/pairing.rs)):

1. The page asks you to get ready, with what to do in Settings, and waits for
   **Start pairing**. A "Pair with siboot" left by an earlier run that was
   killed is withdrawn first.
2. Minis advertises the pairing host under the host's identifier, as the
   iPhone requires, with the seven TXT keys idevice gives. If iOS hasn't let
   Minis use the local network yet (it asks the first time), or Minis's own
   Settings › Network › Local Discovery is off, siboot says so and asks again.
3. A notification says where to find the entry, posted while Minis is still in
   front, so iOS can ask about notifications first if it hasn't.
4. Silence plays through Minis's player (below) while you're in Settings. If
   its sheet is closed or the silence paused while siboot waits, siboot starts
   it again as soon as Minis is in front (`apple-player status`, every 3 s).
5. When the iPhone asks for the code, it shows on the page in large digits and
   comes as a notification. A stray connection or a mistyped code leaves the
   entry up for another tap, three connections in all.
6. The pairing file goes to `~/.siboot` (mode 600), with the host's altIRK, so
   a later pairing keeps the same host identity. The entry is withdrawn.
7. A notification asks you back to Minis. The silence stops once Minis is in
   front again (`minis-bonjour list`), and siboot pair-verifies the new file
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
playback session category, so siboot writes 16 minutes of silence (8 kHz, 8-bit
WAV, 7.7 MB) to `~/.siboot` and plays it until pairing ends and you're back in
Minis ([`src/keepalive.rs`](src/keepalive.rs)). The player opens as a sheet over
whatever Minis shows (the page, or the terminal), and closing it stops the
silence, so siboot says to leave it open, and closes it itself.

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

It needs what the install needs: a pairing the iPhone accepts (made first, the
same way, if there isn't one), and a tunnel that opens. So it can't bring the
switch back on an iPhone that has never been paired, since pairing happens under
that same Developer Mode page. Whether
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
cutting and wrapping, keys as the terminal sends them, code boxes, the pairing
code's large digits, the progress bar, boxes), the log's cleaning (emails,
UDIDs, options without their values), pairing in Minis against a fake iPhone
([`src/fake_iphone.rs`](src/fake_iphone.rs), the iPhone's side of pair-setup:
the handshake, SRP M1–M6 with the code, its identity in M5, checking the host's
M6 signature) and stand-ins for Minis's `minis-bonjour`, `apple-notification` and
`apple-player` (a pairing and the file it leaves, pairing again with the same
host identity, the advertisement's name and TXT keys and its withdrawal, a
leftover one withdrawn first, Local Network refused once and asked again, a
Minis that can't advertise or has no `minis-bonjour`, the silence playing until
Minis is in front again, its player closed and opened again, a mistyped code,
three, a connection dropped before the code, nobody tapping), the hand-off from rootshell
against a stand-in for siboot-pair's side (the right code, a mistyped one, three
wrong ones, something that isn't a pairing file, nothing offered, no code
typed), argument
parsing, `developer-mode` against stand-ins for lockdownd and AMFI (on already,
off, unreadable, a declined restart, the passcode refusal, a hang-up), the bundle walk, 2FA answers, server order, the sign-in errors that
stop it, which teams in a listTeams answer are free or paid and how the
question lists them, the saved Apple Accounts (the order, at most seven,
forgetting one, a hand-edited file, mode 600, the menu's choices), the saved
token (read where `apple_session.rs` keeps it, until ten minutes before its
expiry, one naming another account or unreadable, which of Apple's answers
turn it down) and the accounts that signed in before 0.20.0 starting the list,
install refusals, and rust-core's own `apple_session.rs` and
`error_text.rs` tests. A test also stands in for a rootshell that iOS has
paused: the connection goes through and nothing answers. The page's server is
tested over real loopback connections: paths without the key, another Host,
POSTs that aren't JSON or come from another origin, oversized requests, events
replayed and resumed after a reconnect, one progress event a percent, every
kind of question (an answer for another question refused, an empty one or a
code that isn't six digits asked again, cancelling, getting ready), Stop, and
closing. siboot-pair's own tests, with the same fake iPhone, are in
[`../rootshell`](../rootshell#tests).

`cargo run --example screens` plays pairing in Minis as the page and the live
screen show it (`-- rootshell` plays rootshell's hand-off instead,
`-- teams` asks which of two teams signs, `-- saved` offers two saved Apple
Accounts, `-- saved run-out` one whose saved sign-in has run out, and
`-- moved` and `-- nowhere` look for a pairing service that isn't on 49152).

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

- **0.18.0** pairs in Minis again, now that Minis 1.16, which has
  `minis-bonjour`, is on the App Store: 0.11 and 0.12 paired that way with
  Minis's TestFlight build, and SideInstaller installed after it on an iPhone 16
  on iOS 27 (2026-11-10). The pairing code is 0.12's, on today's page and live
  screen; rootshell's hand-off stays for an older Minis, and is still taken
  first whenever siboot-pair is offering. Tested on macOS and Alpine aarch64
  (96 tests, the stand-ins for Minis's commands under busybox `sh` too), and
  through `examples/screens.rs` as the page at an iPhone's size, on a
  45-column live screen and as plain lines; the static build runs `--help`
  and `check` in Alpine. A test build, installed from the `minis-pairing`
  branch, worked on an iPhone with Open Minis 1.16 from the App Store
  (2026-11-14): it paired in Minis, with no rootshell.
- **0.18.1** stops in an Open Minis older than 1.16 before anything else, with
  its own message (update Open Minis in the App Store) and, on the page, an
  Update Open Minis button that opens the App Store. Before, an older Minis
  went on to step 2 and stopped there, pointing at rootshell, unless a saved
  pairing still worked. No log is offered after it.
- **0.19.0** asks which developer team signs when Apple lists more than one,
  each marked free or paid ([above](#more-than-one-team)); with one team
  nothing changes. Tested on macOS and Alpine aarch64 (102 tests, the new
  ones reading a made-up listTeams answer), and through
  `examples/screens.rs -- teams` as the page at an iPhone's size (the paid
  team picked, and Back), on a 45-column live screen (a digit, return and
  Esc) and as plain lines; `install.sh` installed the static build in
  Alpine, skipped a current copy and refused a tampered one, and the build
  ran `check` there. Not signed in to Apple with an account on two teams yet,
  so the free-or-paid rule is AltSign's, not yet seen against Apple's answer
  here.
- **0.19.1** looks for the iPhone's pairing service on its other ports when
  49152 refuses, or answers as something else ([above](#the-pairing-port));
  with the service on 49152 nothing changes. Tested on macOS (108 tests, the
  new ones against stand-ins on the loopback: the pairing service, a listener
  that takes the connection and says nothing, one that answers something
  else, and closed ports), and from a Mac against an iPhone 16 on the same
  Wi-Fi: started at a closed port and at a silent one, `siboot check` found
  the pairing service on 49152 within 0.1 s, and with 49152 left out, all
  16,382 other ports took 24 s and only 54666 took the connection. Also
  through `examples/screens.rs -- moved` and `-- nowhere`, as the page at an
  iPhone's size and as plain lines. Not run in Minis on an iPhone whose
  service has moved, so how fast it goes through LocalDevVPN isn't known yet.
- **0.20.0** remembers the Apple Accounts that sign in, offers them the next
  time, and signs in with the token saved with one before asking for the
  password ([above](#saved-apple-accounts)). Until now siboot asked for the
  password every time, and `open` didn't use it while the token worked.
  Tested on macOS and Alpine aarch64 (121 tests, the new ones on the list's
  file, the saved token as `apple_session.rs` writes it, and which of Apple's
  answers turn it down), and through `examples/screens.rs -- saved` as the
  page at an iPhone's size (an account picked and signed in without the
  password, one forgotten, Another Apple Account cancelled back to the menu,
  and a new one typed, with the line saying it's kept), on a 45-column live
  screen and as plain lines (`-- saved run-out` too); the static build ran
  `--version` in Alpine. Not signed in to Apple with a saved token yet, so
  siboot's own try of it hasn't met Apple's answers here.
- **0.20.1** signs in again: since 2026-11-20 Apple has answered every
  sign-in through the Mac anisette servers with HTTP 503 (#153), turning away
  any client that calls its system `macOS`, and siboot reported a Mac on
  macOS 26.6. It now reports one on macOS 10.15.7, the newest that still says
  `Mac OS X` (SideInstaller 2.1.1's isideload, through the submodule). It also
  starts on anisette.lillq.me, a pool that hands out a client Apple lets
  through: first, or after the servers that share the last good one's
  anisette identity (and with it the saved token), since lillq's work nowhere
  else. Tested on macOS (121 tests) and through rust-core's `apple_session`
  with a made-up account: `init` through lillq and new.ani.sidestore.io got
  Apple's -20970 where 0.20.0's Mac gets 503. The static build ran
  `--version` in Alpine. Not signed in with a real account yet.

Not tested yet: other iPhones and iOS versions, an Apple Account on more than
one team, one at its certificate or App ID limit (siboot then asks which
certificate to revoke), a second run after the 7-day profile expires, and
signing in with a saved account's token, without the password.
