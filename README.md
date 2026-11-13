# ssboot

Installs SideStore on an iPhone from the iPhone itself: no computer, no paid
certificate. It's siboot (this repository's `main` branch, which installs
SideInstaller) with SideStore in its place, installed and handed its pairing as
SideInstaller installs it.

iOS 27+ is required, with Developer Mode on, Wi-Fi, and LocalDevVPN (App Store)
connected.

1. **Pair, in [rootshell](https://apps.apple.com/app/rootshell-local-terminal-ssh/id6755794662)**
   (App Store). Turn on its Settings › Location Diary Mode › Session Only, then
   run in its local shell, one line at a time:

   ```sh
   curl -fsSLO https://raw.githubusercontent.com/FrizzleM/siboot/ssboot/rootshell/bin/ssboot-pair.wasm
   ```
   ```sh
   ifconfig en0 > ifconfig.txt
   ```
   ```sh
   ./ssboot-pair.wasm
   ```

   Open Settings › Privacy & Security › Developer Mode, tap **Pair with ssboot**
   at the bottom, and enter your passcode and the code ssboot-pair shows. Leave
   rootshell running.

2. **Install, in [Open Minis](https://apps.apple.com/app/id6759188481)** (App
   Store), in its terminal (Minis › Settings › Terminal):

   ```sh
   wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/ssboot/minis/install.sh | sh
   ```

   Type the transfer code rootshell shows, then your Apple Account. If siboot
   has already run in this Minis, ssboot uses the pairing siboot saved, and
   step 1 isn't needed.

3. **Open SideStore.** Trust your Apple Account under Settings › General › VPN &
   Device Management, then sign in to SideStore with it. It already has the
   pairing.

ssboot installs SideStore's nightly: on iOS 27 it's the build that reads the
pairing this iPhone can make. How each part works:
[`minis`](minis) (ssboot) and [`rootshell`](rootshell) (ssboot-pair).
