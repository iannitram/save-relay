# iPhone relay-check test app

This is a device-only acceptance harness for #289, not a production app. It uses a separate `relay-check` game, an app-private saves folder and the fixed remote root `save-relay-ios-check`. Never point it at real game saves. The Nextcloud app password goes through the host credential callback to the app's Keychain; the UI clears its password field when an operation begins. The app does not persist or print the password. The network adapter accepts HTTPS only and does not log headers or request bodies. Keep the displayed envelopes private and remove account details before posting evidence.

## Build and install

1. Download `save-relay-1.0.0-rc.2-aarch64-apple-ios.zip` and `SHA256SUMS` from the [rc.2 release](https://github.com/iannitram/save-relay/releases/tag/v1.0.0-rc.2); verify the zip's SHA-256 against the matching line, then unzip it.
2. In Xcode on a Mac, create an iOS App project (SwiftUI, iOS 15 or later), delete its generated app and content-view Swift files, and add `RelayCheck.swift` from this folder plus `bindings/ios/SaveRelay.swift` from the extracted release. Add the extracted `lib/libsave_relay.a` under **Frameworks, Libraries, and Embedded Content**.
3. Set **Import Paths** (`SWIFT_INCLUDE_PATHS`) to the extracted `bindings/ios` directory. Keep its `module.modulemap` and `../../include/save_relay.h` at the release zip's original relative paths. Set a unique bundle identifier and select the owner's signing team. Select a physical iPhone and build/run. The asset has no simulator slice.
4. In the app, enter the HTTPS Nextcloud address, account and a dedicated app password. Tap **Setup** and confirm the JSON envelope has `ok: true`. The app cannot sign in on the owner's behalf; the owner supplies and enters the credential.

## Round trip

On the Mac, use a test manifest with id `relay-check`, `saves_from: "launcher"`, and `include: ["**/*"]`; use a temporary saves directory holding only `check.txt`. Configure the Mac CLI with the **same** test root `save-relay-ios-check`, preferably under a separate test configuration, then push the test file. Record the `pushed` envelope and `shasum -a 256 check.txt`.

On the iPhone, tap **Pull**. Record the `pulled` envelope and displayed SHA-256; compare to the Mac hash. Change the **Contents for next push** field and tap **Write and push**. Record its envelope and SHA-256. On the Mac, pull into the test directory and compare its file hash to the iPhone's. Paste sanitized envelopes and hashes on #289 and record the release tag and device/OS versions. A conflict or `ok: false` is not a passing round trip.

The automated workflow type-checks this source with the iOS SDK. It does not install or run the app, verify signing or Keychain behavior, or contact Nextcloud. The owner-device acceptance remains open until both directions are demonstrated.
