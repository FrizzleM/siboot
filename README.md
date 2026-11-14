# altboot

Installs [AltStore](https://altstore.io) Classic on an iPhone from the iPhone
itself: no computer, no paid certificate. It's siboot (this repository's `main`
branch, which installs SideInstaller) with AltStore in its place, downloaded,
signed and handed this iPhone's pairing the way SideInstaller installs AltStore.

iOS 27+ is required, with Developer Mode on, Wi-Fi, LocalDevVPN (App Store)
connected, and [Open Minis](https://apps.apple.com/app/id6759188481) 1.16 or
later (App Store).

1. **Run altboot in Open Minis's terminal** (Minis › Settings › Terminal):

   ```sh
   wget -qO- https://raw.githubusercontent.com/FrizzleM/siboot/altboot/minis/install.sh | sh
   ```

   A page opens over the terminal. The first time, it pairs this iPhone with
   itself: tap **Start pairing**, then in Settings › Privacy & Security ›
   Developer Mode tap **Pair with altboot** at the bottom, and enter your
   passcode and the code altboot shows. If siboot or ssboot has run in this
   Minis, altboot uses the pairing it saved instead. Then your Apple Account.

2. **Open AltStore.** Trust your Apple Account under Settings › General › VPN &
   Device Management, then sign in to AltStore with the same account.

3. **Set up Remote AltServer** in AltStore's Settings, to install and refresh
   apps without a computer. AltStore already has this iPhone's pairing, so the
   setup skips pairing. Keep LocalDevVPN connected while it installs or
   refreshes apps.

altboot installs the newest AltStore Classic that AltStore's own source
(apps.altstore.io) lists for this iPhone's iOS, from 2.3 on, and only the file
the source lists. With an Open Minis older than 1.16, pair in rootshell first
([`rootshell`](rootshell)). How each part works: [`minis`](minis) (altboot)
and [`rootshell`](rootshell) (altboot-pair).
