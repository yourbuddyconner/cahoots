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
```

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
  `ChannelRequestsView` and `ChannelMembersView`.

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
then, channel links open in Safari and the web app. `AppModel.openLink` sends a link whose
fragment is `#/ch/<relay host>/<channel id>/<secret>` to the join screen, and any other `/app/`
link to Safari.

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
4. Not tried in the simulator: the camera (there is none), Face ID on a real enclave key, a
   ceremony with codes only between two screens, background refresh, and the icon badge. The
   unit tests cover the signer's keychain path, HTTP against a real station, and the phone crate
   through the bindings.

The first runs failed on the test's own typing (the relay address field kept part of the old
address) and on a label the screen rightly stops showing once step one ends. Neither was the app.
