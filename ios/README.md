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
  `ChannelRequestsView` and `ChannelMembersView`, and for a badge `PairCardView`.
- `NfcTransport.swift`: one Core NFC session per tap, its delegate on its own queue. It passes
  APDUs to the phone crate's chip client, which runs SCP03 in Rust. It waits 30 seconds for a
  card and 5 seconds for each answer.
- The slot key for a paired badge is a keychain item in the `network.cahoots.secrets` service,
  behind user presence, one per badge, so Face ID or the passcode releases it for each tap.

In People, tap the pencil beside any seat to give it a name. Names are private to this phone,
appear in chat and hello words, and are stored in the circle's protected `seat-names.json` file.
An empty name restores the seat number. Deleting a circle also removes its names.
The chat title and Conversation's People and names action both open the roster.

For hello words, both people open each other's seat and compare the hello number before taking
turns reading their own pair. Each person checks the reply against their expected pair without
reading the answer aloud. Next hello advances only the phone on which it was tapped, so both
people advance after a check. If one number is behind, only that phone advances to catch up.

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
call on `Phone`; the screens follow `web/app/channels.js` and use its words. `pollAll` reads
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

Needs the bench badge: the Pico 2 with its SE051 on USB, the antenna switch on, flashed from the
`chip-pairing` branch. The first boot of that firmware wipes the circles whose keys were in the
chip. Make a new circle on it with the bench ceremony or a pile before pairing, because a badge
with no circles cannot pair.

1. On the badge, open "pair a phone" from the list. It shows "hold the phone to the chip, then
   press both".
2. In the app, open Settings and choose "Pair a card". Type a name for this phone, choose "Hold
   the phone to the card", and hold the phone to the badge's antenna. The app shows the badge's
   circles and four words.
3. On the badge, press both. It shows "pair with" the phone's name and four words. Check they
   match the phone's, then press both again. The badge says "paired with" the name and "tap the
   phone again".
4. In the app, choose "Hold the phone again" and hold the phone to the antenna. Face ID asks for
   the key. The app shows "Paired with" the badge and the circles it kept. Choose "Done".
5. On Circles, each of the badge's circles shows the badge's face and "Card circle on" the
   badge. Open one's chat. Post from the browser or another phone and read it here, and post
   back. When the composer is replaced by "Tap your card to catch up", choose it and hold the
   phone to the antenna.
6. On the badge, open "phone" from the list and press both. It says "phone unpaired" and "its
   taps end now". In the app, the next tap, for hello words in People or for a key in the chat,
   says "This card no longer knows this phone."

A tap in step 4 before the badge's presses says "Press both buttons on the badge first, then
hold the phone again." "Forget this card" in Settings deletes the card circles from this phone.
The badge keeps its slot until it is unpaired there.

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
   reply and opens Members; `testReader` launches with `-cahoots-link` and the file's link, opens
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

On 2026-10-31, the phone's own chip client (`crates/phone/src/nfc.rs`, `apdu.rs` and
`scp03.rs`) ran against the real SE051 on the Pico 2, applet 7.2.0. No phone was in the loop: the
laptop ran the client, and a bench build of the firmware passed each APDU to the chip over I2C
in no session, with the chip's pin raised for the exchange, as a phone's tap finds it
(`cahoots-host chip`, frame kinds 32 and 33). `scripts/pair-bench.sh` runs it end to end:

1. The board kept a new circle with the laptop as the two cards under it.
2. In no session, the laptop read the circle's public object, and the applet refused each of
   these with `6986`: a MAC with the circle's root or a hello object, a signature with the seat
   key, an AES key at the phone slot's id, deleting the slot, the owner object or the root,
   reading the inbox and writing the public object.
3. The laptop pressed into "pair a phone", read the pairing object, and wrote a signed, sealed
   inbox as "bench laptop". Both showed the same four words (that run: issue crisp fault
   aware). Both again paired.
4. The laptop opened the phone slot's SCP03 session with its key, at C-MAC, C-ENC, R-MAC and
   R-ENC, asked the applet's version in it, and MACed `chat/gen` with the circle's root. Every
   circle's public object carried a device grant and the laptop's key.
5. The laptop pressed into "phone" and unpaired. The old key no longer opened the slot ("the
   card does not open with this key"), and every public object named no phone and held a
   pending bump.

What the applet does that the client and the software chip had to learn: no Le inside a wrapped
command (it ends the session with `6982`), `6985` for an id that holds nothing, the two-byte
TLV length in some answers, and a 2-byte answer to ReadSize. Not tried: Core NFC on a device.

## First-use guide

The app shows a four-page guide on first use. It explains circles and seats, hello words, channel access, and local storage. Completing or skipping it records `cahoots.welcomeSeen` on that phone or browser. The guide can be reopened from **How Cahoots works** in iOS Settings or the browser home screen’s More options menu. Incoming routes are kept while it is shown.

The iOS guide uses small decorative cahoot animations and respects Reduce Motion. Pages return to the top when advancing, including at larger text sizes. The full user guide is [Getting started with Cahoots](../docs/getting-started.md).
