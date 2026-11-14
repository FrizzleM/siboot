# ssboot-pair for rootshell

ssboot's pairing step, as a WASM program for
[rootshell](https://apps.apple.com/app/rootshell-local-terminal-ssh/id6755794662)
("rootshell - Local Terminal & SSH", free, App Store). It pairs the iPhone with
itself, then hands the pairing file to ssboot in Open Minis
([`../minis`](../minis)), which signs and installs SideStore.

It's siboot-pair 0.4.0 (this repository's `main` branch) with ssboot's name and
install command: Settings shows **Pair with ssboot**, and it tells you to run
ssboot in Minis. What it keeps in rootshell's folder (`rp_pairing_file.plist`,
`host_alt_irk`) and how it hands the file over are siboot-pair's, so the host
identity either one made is kept, and siboot and ssboot each take the file from
either.

Since Open Minis 1.16 (App Store, 2026-11-14), ssboot 0.3.0 and later pair the
iPhone in Minis themselves ([`../minis`](../minis#pairing-in-minis)), so
ssboot-pair is only needed with an older Minis. ssboot still takes its file
whenever it's offering one.

## Running it

Needs iOS 27 or later, Developer Mode on, Wi-Fi, and rootshell allowed Local
Network access. In rootshell, turn on **Settings › Location Diary Mode ›
Session Only**: it keeps rootshell running while you're in Settings. Then, in
its local shell, one line at a time (rootshell doesn't run them joined with
`&&`):

```sh
curl -fsSLO https://raw.githubusercontent.com/FrizzleM/siboot/ssboot/rootshell/bin/ssboot-pair.wasm
ifconfig en0 > ifconfig.txt
./ssboot-pair.wasm
```

Open **Settings › Privacy & Security › Developer Mode**, scroll to the bottom,
tap **Pair with ssboot**, and enter your passcode and the 6-digit code
ssboot-pair shows in large digits (it also arrives as a notification). Leave
rootshell running, and in Open Minis's terminal run:

```sh
wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/ssboot/minis/install.sh | sh
```

The first time, ssboot asks for the transfer code ssboot-pair shows (in large
digits, and as a notification you can pull down in Minis) and takes the pairing
file. After that it uses the file it saved and doesn't ask, so ssboot-pair keeps
offering until you close its tab. If siboot has already run in that Minis,
ssboot uses the pairing file siboot saved, and this step isn't needed.

ssboot-pair's screen is lines that stay and one live line at the bottom, which
says what it's waiting for and is rewritten in place: a WASM program in
rootshell can't learn the terminal's size and blocks in every socket call, so
that line is kept short enough never to wrap, and moves (a spinner) only while
the program is awake, during the announcement. Lines are wrapped for an iPhone
upright, about 40 columns, or to `COLUMNS` when it's set. `--plain` prints plain
lines instead.

When it stops on a problem, ssboot-pair asks **Copy the log to the clipboard?
y/n**: type `y` (or nothing) and return. A WASM program can't run a command, so
the copy is OSC 52, which rootshell's terminal (Ghostty) puts on the clipboard;
nothing tells ssboot-pair whether it did, so the log is also saved as
`ssboot-pair.log` in rootshell's folder (Files › rootshell). The log is the
lines it showed, plainly, without the codes. It has no times: reading the clock
would add a WASI import the module doesn't have.

## How it works

iOS 27 lists a host under Settings › Privacy & Security › Developer Mode ›
"Pair with …" when it advertises `_remotepairing-pairable-host._tcp`, and then
dials it to pair. An app may only advertise the Bonjour types its Info.plist
declares, unless it has Apple's multicast entitlement, which lets it send mDNS
itself. rootshell has it (`com.apple.developer.networking.multicast`), runs WASI
Preview 1 programs, and lends them sockets (`rootshell_socket_*`).

1. Reads this iPhone's Wi-Fi address from `ifconfig.txt`. A WASM program can't
   find it out (rootshell's `getsockname` always says 0.0.0.0), and the
   announcement needs it for its A record.
2. Listens on a random port and sends the announcement (PTR, SRV, TXT with the
   seven keys idevice's `PairableHostInfo` gives, and A) three times, each on a
   fresh UDP socket, with a 75-minute TTL. It can't answer queries, as the ABI
   has no poll and the program then blocks in `accept`, so Settings is answered
   from mDNSResponder's cache.
3. Pairs with idevice's `PairableHost`, the responder SideInstaller uses. The
   code is printed in large digits and posted as a notification (OSC 777, which
   rootshell's terminal turns into one, also in the background). A stray connection or a
   mistyped code leaves the entry up for another tap, three connections in all.
4. Writes `rp_pairing_file.plist` and `host_alt_irk` into rootshell's folder,
   so a second run keeps the same host identity, and withdraws the announcement.
5. Offers the file on the iPhone's own loopback, port 61207, behind a 6-digit
   transfer code, which ssboot in Minis asks for, shown and posted the same
   way. The code keeps other apps on the iPhone from taking the file: five
   wrong ones end the offer. The request line is still `SIBOOT1 <code>`, as
   siboot sends it.

## Building

idevice comes from the [`SideInstaller`](../SideInstaller) submodule, so check
it out first (`git clone --recursive`, or `git submodule update --init`):

```sh
rustup target add wasm32-wasip1
cargo build --release --target wasm32-wasip1
cp target/wasm32-wasip1/release/ssboot-pair.wasm bin/
```

idevice's `wasm` feature is meant for browsers, but on WASI its randomness
shims fall back to WASI's `random_get` and none of its JS shims are linked: the
module imports only `wasi_snapshot_preview1` and `rootshell_socket`.

## Tests

`cargo test` runs on a Mac. There the socket calls are served by the Mac's own
sockets (TCP on loopback only; announcements are dropped, so nothing reaches
the network), so the tests run the real pairing and hand-off code.
[`src/fake_iphone.rs`](src/fake_iphone.rs) plays the iPhone's side of
pair-setup against it: the handshake, SRP M1–M6 with the code, and its identity
in M5, checking the host's M6 signature. They cover a pairing and the file it
leaves, a mistyped code, three wrong codes, a connection that drops before the
code, the hand-off (the file only for the right code, five wrong ones ending the
offer), the announcement's records, the address out of `ifconfig` output, and
the log (no codes in it) and its base64 for OSC 52.

## What's been tested

- **0.1.0** is siboot-pair 0.4.0 with ssboot's name and install command; the
  pairing and the hand-off are unchanged. Tested with `cargo test` on a Mac (16
  tests). Its `.wasm` imports the same 23 functions as siboot-pair 0.4.0's (15
  from `wasi_snapshot_preview1`, 8 from `rootshell_socket`). Not yet run on an
  iPhone.

siboot-pair's own history, up to 0.4.0, is in its README on the `main` branch:
0.1.0 paired on an iPhone (iOS 27) and siboot 0.12.0 installed SideInstaller
with the file.
