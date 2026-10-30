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
- `UrlSessionHttp.swift`: blocking GET and POST with a 25 second timeout.
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
- The screens: `CirclesView`, `NewCircleView`, `JoinView`, `CeremonyView` (with `CeremonyModel`),
  `ChatView`, `PeopleView` and `HelloView`, `SettingsView`.

`Info.plist` is written by hand and preprocessed. Debug builds allow plain HTTP to a local relay
(`NSAllowsLocalNetworking`) so a `cahoots-station` on the Mac works; Release builds do not. The
export compliance question (`ITSAppUsesNonExemptEncryption`) is left to be answered at upload,
since the app encrypts messages itself. `PrivacyInfo.xcprivacy` declares no data collected and
the two required-reason APIs the Rust standard library uses: file timestamps (C617.1) and disk
space (E174.1).

Every scan screen has a field to paste a code under the camera, for the simulator and for anyone
who cannot use the camera.

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
