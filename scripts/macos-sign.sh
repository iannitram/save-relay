#!/usr/bin/env bash
# Sign and notarize the macOS release binary (#239, part of #238).
#
# Usage: scripts/macos-sign.sh <binary>
#
# With the Apple secrets set, the binary is signed with the hardened runtime and a secure
# timestamp, the signature is checked, the binary is sent to Apple's notary service and waited
# for, and it is run once more (`version`) to prove it still starts. Without them it is left
# unsigned and the script says so; on a tag (GITHUB_EVENT_NAME=push) that is a warning.
#
# Environment (repo secrets, read by .github/workflows/release.yml):
#   MACOS_CERTIFICATE_P12_BASE64  the Developer ID Application certificate and key, .p12, base64
#   MACOS_CERTIFICATE_PASSWORD    the .p12's password
#   MACOS_SIGNING_IDENTITY        optional: the identity's name or SHA-1; default: the first
#                                 "Developer ID Application" identity in the imported certificate
#   APPLE_ID                      the Apple Account that notarizes
#   APPLE_TEAM_ID                 its team id
#   APPLE_APP_PASSWORD            an app-specific password for that account
#   RUNNER_TEMP                   where the throwaway keychain goes (set by GitHub Actions)
#
# A bare binary cannot carry a stapled ticket (only apps, disk images and installer packages
# can), so Gatekeeper checks it online on first run; notarization is what makes it pass.
#
# The keychain set-up, the codesign flags and the notarytool call are adapted from starship
# (https://github.com/starship/starship): .github/workflows/release.yml at
# 94dfccf79eeb6b79805dcfbdcc2d788f6fb4c248 (job notarize_and_pkgbuild) and
# install/macos_packages/build_and_notarize.sh at 6e6664dea641b937e0999368d83819da6737bc64.
# Their notice:
#
#   ISC License
#
#   Copyright (c) 2019-2022, Starship Contributors
#
#   Permission to use, copy, modify, and/or distribute this software for any
#   purpose with or without fee is hereby granted, provided that the above
#   copyright notice and this permission notice appear in all copies.
#
#   THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
#   WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
#   MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
#   ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
#   WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
#   ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
#   OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

set -euo pipefail

if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
  echo "usage: $0 <binary>" >&2
  exit 2
fi
binary="$1"

if [ -z "${MACOS_CERTIFICATE_P12_BASE64:-}" ]; then
  if [ "${GITHUB_EVENT_NAME:-}" = "push" ]; then
    echo "::warning::MACOS_CERTIFICATE_P12_BASE64 is not set: the macOS binary is unsigned (#238)"
  else
    echo "macos-sign: no signing certificate set; $binary stays unsigned"
  fi
  exit 0
fi

for name in MACOS_CERTIFICATE_PASSWORD APPLE_ID APPLE_TEAM_ID APPLE_APP_PASSWORD RUNNER_TEMP; do
  if [ -z "${!name:-}" ]; then
    echo "::error::$name is not set, but MACOS_CERTIFICATE_P12_BASE64 is: set all of the signing secrets or none"
    exit 1
  fi
done

keychain="$RUNNER_TEMP/save-relay-signing.keychain-db"
certificate="$RUNNER_TEMP/save-relay-signing.p12"
archive="$RUNNER_TEMP/save-relay-notarize.zip"
# The keychain's own password only lives for this run.
keychain_password="$(openssl rand -hex 24)"

cleanup() {
  security delete-keychain "$keychain" 2>/dev/null || true
  rm -f "$certificate" "$archive"
}
trap cleanup EXIT

echo -n "$MACOS_CERTIFICATE_P12_BASE64" | base64 --decode > "$certificate"

# A temporary keychain, unlocked for six hours, searched first, so codesign finds the identity
# without a prompt.
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$certificate" -P "$MACOS_CERTIFICATE_PASSWORD" -A -t cert -f pkcs12 -k "$keychain"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain"

identity="${MACOS_SIGNING_IDENTITY:-}"
if [ -z "$identity" ]; then
  identity="$(security find-identity -v -p codesigning "$keychain" \
    | sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -n 1)"
  if [ -z "$identity" ]; then
    echo "::error::no Developer ID Application identity in the certificate"
    exit 1
  fi
fi

echo "macos-sign: signing $binary"
codesign --timestamp --keychain "$keychain" --sign "$identity" --options runtime --force --verbose "$binary"
codesign --verify --strict --verbose=2 "$binary"

echo "macos-sign: notarizing"
ditto -c -k --keepParent "$binary" "$archive"
result="$(xcrun notarytool submit "$archive" \
  --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD" \
  --wait --output-format json)"
echo "$result"
# notarytool can finish waiting on an Invalid submission; only Accepted is a pass.
case "$(printf '%s' "$result" | tr -d ' \n')" in
  *'"status":"Accepted"'*) ;;
  *)
    echo "::error::notarization was not accepted (see the result above; notarytool log <id> has why)"
    exit 1
    ;;
esac

# Signing rewrites the file: it must still start.
"$binary" version
echo "macos-sign: $binary is signed and notarized"
