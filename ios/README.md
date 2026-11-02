# Cahoots for iOS

A SwiftUI app over `crates/phone`, the app's whole state machine in Rust behind UniFFI. Swift
holds the views, the camera, the Secure Enclave keys, HTTP to the relay and file protection.
Nothing cryptographic happens in Swift. The design is
`docs/superpowers/specs/2026-10-30-ios-app-design.md`.

## Build

Needs Xcode 15.4 or later, XcodeGen (`brew install xcodegen`) and the Rust iOS targets
(`rustup target add aarch64-apple-ios aarch64-apple-ios-sim`).

```
cp ios/Signing.xcconfig.example ios/Signing.xcconfig   # then put your team id in it
scripts/ios-build.sh          # the Rust library, the Xcode project, the app for the simulator
scripts/ios-build.sh test     # the same, then the XCTest suites against a local cahoots-station
scripts/ios-walk.sh OUT       # two simulators hold a ceremony through a local relay, then chat
scripts/ios-channel-walk.sh WALK OUT   # then one starts a channel and the other joins it by link
```

A debug build also takes `-cahoots-link <url>` and treats it as a channel link tapped outside the
app, which is how the channel walk hands the reader its link on a simulator.

`ios/Cahoots.xcodeproj` is generated from `ios/project.yml` and is not checked in. The project
reads the library and the generated Swift from `target/ios`, which `scripts/ios-build.sh` links
to `$CARGO_TARGET_DIR/ios` when the target directory is elsewhere.

## How the Swift side is laid out

- `Core.swift`: the one `Phone`, and the serial queue every call to it goes through. `Phone`
  holds a lock across its HTTP and signer callbacks, so nothing calls it on the main thread. The
  camera calls `decode_frame` from its own queue, since that call reads no state.
- `EnclaveSigner.swift`: keys made in the Secure Enclave with user presence, their handles in
  the keychain (`WhenUnlockedThisDeviceOnly`). On the simulator, a CryptoKit key in the keychain.
- `UrlSessionHttp.swift`: blocking GET and POST with a 25 second timeout, and `request` (any
  method and headers) for the channel calls, which returns the status rather than throwing.
- `PhoneStore.swift`: Application Support/cahoots, left out of backups. `circles/` has complete
  protection. `phone.cbor` and each circle's `cursor.cbor` and `settings.cbor` hold nothing secret
  and get protection until first unlock, set again after every poll.
- `BackgroundRefresh.swift`: `network.cahoots.refresh` polls every circle, each for at most 20
  seconds, and sets the icon's count. iOS decides when it runs. Polls while the phone is locked
  fail: the circle records have complete protection and cannot be read then. So the badge
  updates only when a refresh runs while the phone is unlocked, and a failed or partial pass
  leaves the list and the badge as they were rather than showing zero.
- `OpenCeremonies.swift`: a ceremony lives only in the phone crate's memory. Its id is kept in
  UserDefaults while it is open, so a ceremony lost when the app closed is reported once at the
  next launch ("This ceremony was lost when the app closed. Everyone starts over with a new
  code.") and the seat key made for it is deleted.
- The look: `Theme.swift` (colours, type, buttons, sections) and `FaceView.swift` (a seat's
  cahoot, drawn from `Phone.facePixels`). `DESIGN.md` has the plan.
- The screens: `CirclesView`, `NewCircleView`, `JoinView`, `CeremonyView` (with `CeremonyModel`),
  `ChatView`, `PeopleView` and `HelloView`, `SettingsView`, and for channels `ChannelsView`
  (the list section and the start sheet), `ChannelView`, `JoinChannelView`,
  `ChannelRequestsView` and `ChannelMembersView`, and for a badge `PairCardView` and
  `CardView`, the Card screen with its one button.
- `NfcTransport.swift`: one Core NFC session per tap, its delegate on its own queue. It passes
  APDUs to the phone crate's chip client, which runs SCP03 in Rust. It waits 30 seconds for a
  card and 5 seconds for each answer.
- The slot key for a paired badge is a keychain item in the `network.cahoots.secrets` service,
  behind user presence, one per badge, so Face ID or the passcode releases it for each tap after
  pairing.

In People, each seat shows the name and face its owner shared. A seat without a shared name
has a pencil beside it: tap it to give that seat a name only this phone sees. These names
appear in chat and hello words and are stored in the circle's protected `seat-names.json`
file. An empty name restores the seat number. Deleting a circle also removes its names.
The chat title and Conversation's People and names action both open People.

For hello words, both people open each other's seat and compare the hello number before taking
turns reading their own pair. Each person checks the reply against their expected pair without
reading the answer aloud. Next hello advances only the phone on which it was tapped, so both
people advance after a check. If one number is behind, only that phone advances, until the
numbers match.

`Info.plist` is written by hand and preprocessed. Debug builds allow plain HTTP to a local relay
(`NSAllowsLocalNetworking`) so a `cahoots-station` on the Mac works. Release builds do not. The
export compliance question (`ITSAppUsesNonExemptEncryption`) is left to be answered at upload,
since the app encrypts messages itself. `PrivacyInfo.xcprivacy` declares no data collected and
the two required-reason APIs the Rust standard library uses: file timestamps (C617.1) and disk
space (E174.1).

Every scan screen has a field to paste a code under the camera, for the simulator and for anyone
who cannot use the camera.

## Channels

A channel posts to everyone who has its link. The circles screen lists the channels on this
phone and, for each circle, "Start a channel" (a name and one of five policies). The channel
screen shows the posts and a composer for those who may post, and reads the relay every three
seconds while it is open. The owner's menu shares the link, retires it, starts new keys, and
opens Members (revoke, which shows the new link) and Requests (allow or refuse, and a toggle to
allow automatically what the policy permits, kept in UserDefaults). Every step is a `channel_*`
call on `Phone`. The screens follow `web/app/channels.js` and use its words. `pollAll` reads
every live channel after the circles, and channel unread counts add to the icon badge.

## Universal links

`Cahoots/Cahoots.entitlements` (written by XcodeGen from `project.yml`) holds
`com.apple.developer.associated-domains` with `applinks:cahoots.network`. iOS gives the app a
link under `https://cahoots.network/app/` only once the site serves
`/.well-known/apple-app-site-association` naming the app, which happens when main deploys. Until
then, channel links open in Safari and the web app. `AppModel.linkAction` decides what a link
does. A link whose fragment is `#/ch/<relay host>/<channel id>[/<secret>]` opens the join screen
(a preview read from the relay, then Open), or the channel itself when this phone already holds
it. The screen goes on top of the current one, and a link that arrives during a ceremony waits
until the ceremony is done or left. Any other `/app/` link goes to Safari.

## Ship to TestFlight

### Once

1. Install Xcode 26 or later with the iOS platform. App Store Connect refuses builds made with an older SDK. Sign into the developer account in Xcode (Settings, Accounts). Team id `9Z585PW7QR`.
2. `cp ios/Signing.xcconfig.example ios/Signing.xcconfig` and set `DEVELOPMENT_TEAM` to the team
   id. The file is git-ignored.
3. In App Store Connect, create the app record: My Apps, New App, platform iOS, name Cahoots,
   bundle id `network.cahoots.app`. The App ID itself already exists: the first archive made it
   with automatic signing.
4. In App Store Connect, Users and Access, Integrations, make an API key with the App Manager
   role. Save the downloaded file as `~/.appstoreconnect/private_keys/AuthKey_<key id>.p8`, and
   note the key id and the issuer id.

### Every release

```
ASC_KEY_ID=<key id> ASC_ISSUER_ID=<issuer id> scripts/ios-archive.sh
```

The script builds the Rust library, generates the project, archives a Release build, exports
`Cahoots.ipa` signed for the App Store, and uploads it with `xcrun altool`. The build number is
the UTC date and time (`YYYYMMDDHHMM`). The version is `MARKETING_VERSION` in `project.yml`.
`--no-upload` stops after the export. Without the two variables and the key file, the script
exports and says how to upload (the key, or Transporter with the `.ipa`).

The first time a build reaches TestFlight, App Store Connect asks about export compliance.
Answer that the app uses non-exempt encryption: it encrypts messages itself, with ChaCha20.
Classify it yourself as mass-market encryption, and file the annual self-classification report
with BIS (the US Bureau of Industry and Security) each year the app is available.

Tried on 2026-10-30: `scripts/ios-archive.sh --no-upload` archived build 202610302129 and
exported the `.ipa`. Xcode made an Apple Development certificate on this Mac, and the export is
signed "Apple Distribution: Conner Swann (9Z585PW7QR)" with the profile "iOS Team Store
Provisioning Profile: network.cahoots.app". The upload was not tried: it needs the app record
and the API key above. Later that day, with Xcode 27.2 and the iOS 27.2 SDK, the script uploaded
build 202610302331 to App Store Connect (delivery 462fbaa7).

### Pair a card on a device

Needs the bench badge: the Pico 2 with its SE051 Click board on USB, flashed from main. The
chip takes its antenna only when the phone's field is what starts it, so whenever a phone may
tap, the badge cuts the Click board's power. For that the bench needs two things: the Click
board's ENA SEL switch on EN (not ON), so the badge's enable pin reaches the chip, and the
Click board's +3.3V fed through a PNP transistor (an S8550 works) with its emitter on the
Pico's 3V3, its collector on the Click board's +3.3V pin and its base through about 1 kΩ to
GP3, the Pico's pin 5. The antenna switch is on. Without the transistor the chip stays powered
and no tap reaches it. Make a new circle on the badge with the bench ceremony or a pile before
pairing, because a badge with no circles cannot pair. The two circles already on the bench
badge, owner-bench3 and owner-bench4, were kept before the badge made hello objects, so their
seats get no hello words from the card: People says "This seat has no hello words from the
card." for them, and the Card screen does not count them. To try hello words from the card,
keep a new circle, before pairing or after: a circle the badge keeps after pairing comes to the
phone at its next tap on the Card screen.

The badge rests on home: "badge" over the names of its circles, a status line, and "tap",
"A+B menu" and "menu" along the bottom. The left button opens a tap window of two minutes: the
status line says "open for a tap, 2 min", then "1 min", and the chip's power is cut until the
window closes. While the window is open the left button's label says "close". Press it again
to close the window early, and the status line says "closed to taps". Opening the menu closes
it too. Both buttons, or the right one, open the menu: the badge's circles by name, "new
circle", "pair a phone", "phone" while a phone is paired, and "back". The side buttons move
along it and both buttons choose. A note after an action goes home by itself after five
seconds, or at any press.

1. On the badge, press both for the menu, move to "pair a phone" and press both. The page says
   "hold the phone to the chip, then press both". The chip's power is cut while it is open.
2. In the app, open Settings and choose "Pair a card". Type a name for this phone and choose
   "Tap the card". Face ID asks first, before the NFC sheet, because the phone signs its
   request with its enclave key during the tap and the camera cannot see a face while the
   phone lies on the badge. Then hold the phone to the badge's antenna. The app shows how
   many circles the badge holds and six words, and says "Set the phone down. On the badge, press
   both to show its six words. Compare every word in order. If they all match, press both
   again, then hold the phone to the card." A badge with no name shows as "your badge".
3. On the badge, press both. It shows "pair with" the phone's name and six words in two rows.
   Check they match the phone's, then press both again. The badge says "paired with" the name
   and "hold the phone again", then goes home with the tap window open for that tap.
4. In the app, choose "Tap the card again" and hold the phone to the antenna. This tap asks
   nothing. It brings each of the badge's circles onto the phone with its chat key and the
   words for the next eight hello numbers with each other seat. The app shows "Paired with" the
   badge and the circles it kept. Choose "Done". The badge took four presses of both buttons in all.
5. On Circles, each of the badge's circles shows the badge's face and "Card circle on" the
   badge. Open one's chat. Post from the browser or another phone and read it here, and post
   back.
6. In Settings, "Card" opens the Card screen: the badge's name, when it was last tapped, what a
   tap would do now ("up to date", "1 circle needs a key", "hello words: 2 left for Ada"), and
   one button, "Tap the card". To tap, press the left button on the badge so home says "open
   for a tap", then choose "Tap the card" and hold the phone to the antenna. No page on the
   badge needs opening. Face ID asks before the NFC sheet, because the tap reads the card's key
   from the keychain. One tap fetches every key a chat waits for, passes on the bumps the badge
   signed, and tops up the words to eight hello numbers with each other seat. It also keeps
   any circle the badge kept since it paired, with its chat key and hello words, and the result
   says "and kept 1 new circle". When a chat's composer is replaced by "Tap the card", that
   button taps the same way.
7. In People, open a seat for hello words. They come from the phone with no tap, as in any
   other circle. When three or fewer are left with that seat, the page says "N hellos left from
   your card. Tap the card soon." When none are left, "Next hello" becomes "Tap the card", which
   taps as in step 6. A circle the badge kept before it made hello objects, before the evening
   of 2026-10-31, gets no hello words from the card: its seats say "This seat has no hello
   words from the card." and offer no tap.
8. On the badge, press both for the menu, move to "phone" and press both. The page names the
   paired phone. Press both again to unpair. The badge says "phone unpaired" and "its taps end
   now", then goes home. In the app, the next tap says "This card no longer knows this phone.
   Choose Forget this card, then pair again." The card circle still reads and posts in its chat
   until a phone posts the bump the badge signed at the unpair. If a phone is lost, unpair it
   on the badge, then pair another phone, which posts the bump, or ask a member with the
   circle's root to reset the chat. Neither ends the hello words the lost phone holds, since
   they do not depend on the chat. Ask the other members to move their hello number with the
   badge's seat past the next eight, and to treat the lost phone's words as spent. To pair this
   phone again, choose "Forget this card" on the Card screen, then "Pair a card" in Settings and
   "pair a phone" on the badge. If the app still shows the words from before, choose "Start
   over" first: those words were for a pairing the badge has closed.

A tap in step 4 before the badge's presses says "Press both buttons on the badge first, then
tap the card again." "Forget this card" on the Card screen deletes the card circles from this
phone. The badge keeps its slot until it is unpaired there. A phone keeps one badge: to pair
another, forget this one first. Removing one card circle in Settings deletes it from this phone
only: the next tap on the card brings it back, unless the badge forgets it first.

### What to try first on a device

- The camera: scan a host's code to join, and as host scan a joiner's code. Then a codes-only
  ceremony between two phones, where both show and scan animations.
- Face ID at the ceremony's signature: the prompt, "Confirm with Face ID" on screen, and what a
  cancelled prompt does ("Sign with Face ID" to try again).
- Background refresh and the badge: leave a chat with the app closed and the phone unlocked,
  have someone send, and see whether the icon's count changes. iOS decides when refresh runs.
- Settings should say "Seat key in the Secure Enclave, secret protected by iOS." with no
  simulator note.

## Tried in the simulator

On 2026-10-30, Xcode 15.4, iOS 17.5, on a Mac with Apple silicon.

1. `scripts/ios-build.sh`, then `xcrun simctl boot "iPhone 15"`,
   `xcrun simctl install booted <DerivedData>/Build/Products/Debug-iphonesimulator/Cahoots.app`,
   `xcrun simctl launch booted network.cahoots.app` and `xcrun simctl io booted screenshot`.
   The circles screen rendered: "Meet in person. Stay in cahoots.", "Create a circle",
   "Join a circle", the settings button, and "Circles and history are saved on this phone."
2. `xcrun simctl clone "iPhone 15" "iPhone 15 Walk B"` and booted both, then
   `scripts/ios-walk.sh OUT`. It starts a `cahoots-station` on a free local port and runs the UI
   test `RelayWalkUITests` on both simulators at once, `testHost` on one and `testJoiner` on the
   other. Each side taps through the app's own screens. A simulator has no camera, so each side
   reads the code on its screen (a screenshot of the drawn QR code, decoded with Core Image),
   writes it to a file in OUT, and the other side types it into the "Paste a code" field.
3. What happened, in order. The host opened Settings, which said "This simulator has no Secure
   Enclave. The key is in the keychain." and showed the phone's key fingerprint, set the relay for
   new circles to the local station, and created a circle "park" for 2 people through the relay.
   It showed "Show this code to everyone joining" with the offer code and "Seat 2 · waiting". The
   joiner pasted the offer, saw "Which seat are you?" with seat 2 picked, chose "Make my
   contribution", and showed "Show this to the host" with its join code. The host pasted that
   code under "Scan a joiner's code", saw "Scanned seat 2" and "Everyone is scanned", and chose
   Continue. Both reached "Step 4 of 4 · Compare words" with the same six words (that run:
   build manual gold agree monster butter), chose "They match", and landed in the chat. The host
   sent "hello from the host", the joiner saw it from seat 1 in epoch 1 and answered "hello from
   the joiner", and the host saw that. The station's log held 10 entries. Both tests passed, in
   about 48 seconds each.
4. On 2026-10-31, after that walk, `scripts/ios-channel-walk.sh WALK OUT` with WALK the folder
   the relay walk wrote. It starts a station with the channel features on the same port, so the
   circles' relay address still works, and runs `ChannelWalkUITests` on both simulators:
   `testOwner` starts a "Garden news" group chat from the circle, shares the link (read off the
   share screen and written to a file in OUT), posts, lets the reader in from Requests, reads the
   reply and opens Members. `testReader` launches with `-cahoots-link` and the file's link, opens
   the channel from the join screen, reads the first post, asks to post as "Bea", waits for the
   grant, and posts. Each step is photographed into OUT.
5. On 2026-10-31, a ceremony a phone hosts with a badge through the bridge:
   `scripts/bridge-bench.sh --sim`. The host simulator ran `testHost` with
   `TEST_RUNNER_CAHOOTS_CARDS=1`: it made "park" for 3 people with 1 card in the pile, and its
   seat list showed "Seat 2, card, through the bridge". The script started a simulated badge
   (`cahoots-host badge`, the badge crate on a laptop) and `cahoots-host bridge --sim` on its
   socket with the host's code. The bridge printed the badge's join code, and the script wrote
   it to `card-2.txt` for the host to paste. The host saw "Scanned seat 2", pasted the joiner's
   code from the other simulator and saw "Scanned seat 3", then chose Continue. The badge took
   seat 2 of 3. The host, the badge and the joiner showed the same six words (that run: night
   vanish sound capable clean emerge), the badge kept the circle, and the host and joiner chatted.
   The station held 14 entries. The same script without `--sim`, with the Pico 2 and its SE051
   on `/dev/cu.usbmodempico21` flashed with the bench build, ran on 2026-10-31: the board took
   seat 2, the host, the board and the joiner showed the same six words (spy shallow young
   weather afford island), the board kept the circle, and the station held 12 entries.
6. Not tried in the simulator: the camera (there is none), Face ID on a real enclave key, a
   ceremony with codes only between two screens, background refresh, and the icon badge. The
   unit tests cover the signer's keychain path, HTTP against a real station, and the phone crate
   through the bindings.

The first runs failed on the test's own typing (the relay address field kept part of the old
address) and on a label the screen rightly stops showing once step one ends. Neither was the app.

## Tried on the bench

On 2026-10-31 and twice on 2026-11-01, the phone's own chip client (`crates/phone/src/nfc.rs`,
`apdu.rs` and `scp03.rs`) ran against the real SE051 on the Pico 2, applet 7.2.0. No phone was
in the loop: the laptop ran the client, and a bench build of the firmware passed each APDU to
the chip over I2C in no session, with the chip's power on for the exchange, as a phone's tap
finds it (`cahoots-host chip`, frame kinds 32 and 33). Where the badge has cut the chip's power
for a phone's tap, the bench build gives it back for the laptop's exchange, since the laptop
reaches the chip by wire. `scripts/pair-bench.sh` runs it end to end, pressing the badge's
buttons from the laptop. The last run, on 2026-11-01 with the badge resting on home:

1. From home the laptop pressed both for the menu and chose "new circle", and the board kept a
   new circle with the laptop as the two cards under it. The badge said "saved" and went home.
2. In no session, the laptop read the pairing object, which named circle slots 0, 1 and 2, and
   the applet refused to give it any of their public objects. It refused each of these with `6986`
   too: a MAC with the circle's root or a hello object, a signature with the seat key, an AES
   key at the phone slot's id, deleting the slot, the owner object or the root, reading the
   inbox, and reading or writing the public object.
3. The laptop pressed into "pair a phone" and ran the app's own pairing code as "bench laptop",
   with no network (`cahoots-host chip pair`). The first tap wrote a signed, sealed inbox. Both
   showed the same six words (empower estate dad enlist inner citizen), and both again paired.
4. The second tap kept the badge's three circles in 6.2 seconds: 83 APDUs, of which 32 were
   hello MACs for the new circle's two other seats, 11 were root MACs for the chat keys and the
   bumps that waited, and 31 were reads. The two older circles were kept before the badge made
   hello objects, so their seats got no words from the card: one refused MAC each, and the tap
   went on.
5. With the key from that tap the laptop opened the phone slot's SCP03 session, at C-MAC,
   C-ENC, R-MAC and R-ENC, asked the applet's version in it, and MACed `chat/gen` with the new
   circle's root. In that session every public object the pairing object names carried a
   device grant and the laptop's key.
6. The laptop pressed into "phone" and unpaired. The old key no longer opened the slot ("the
   card does not open with this key"), and the badge's log said it wrote every public object
   with a pending bump and no phone. Then it forgot the new circle through the badge's own
   pages. The run ended "pair bench: ok".

The bench chip was owned under the owner secret of builds before 2026-11-01. The firmware tries
that older secret when the current one does not open the chip, and logs "se051: owner session
open with the older secret".

What the applet does that the client and the software chip had to learn: no Le inside a wrapped
command (it ends the session with `6982`), `6985` for an id that holds nothing, the two-byte
TLV length in some answers, and a 2-byte answer to ReadSize. Core NFC on a device first reached
this chip on 2026-10-31, an iPhone 11 held to the board's antenna while the board had no power.
A phone first paired with this board by tap on 2026-11-01.

## First-use guide

On first use, the iOS app shows four guide pages, then asks for your name on a fifth page. The guide explains circles and seats, hello words, channel access, and local storage. Completing or skipping it records `cahoots.welcomeSeen` on that phone. **How Cahoots works** in Settings reopens the four guide pages. The browser also has a guide, opened from the home screen’s More options menu. Incoming routes are kept while the iOS guide is shown.

The iOS guide uses small decorative cahoot animations and respects Reduce Motion. Pages return to the top when advancing, including at larger text sizes. The full user guide is [Getting started with Cahoots](../docs/getting-started.md).
