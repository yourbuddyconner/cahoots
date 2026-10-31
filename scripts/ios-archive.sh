#!/usr/bin/env bash
# Archive the iOS app for the App Store, export the .ipa, and upload it to TestFlight.
#
#   scripts/ios-archive.sh               archive, export, upload (when the API key is set)
#   scripts/ios-archive.sh --no-upload   archive and export only
#
# Needs ios/Signing.xcconfig with DEVELOPMENT_TEAM (copy Signing.xcconfig.example), Xcode signed
# into that team, and for the upload an App Store Connect API key:
#   ASC_KEY_ID and ASC_ISSUER_ID set, the key at ~/.appstoreconnect/private_keys/AuthKey_<id>.p8
# The build number is the UTC date and time, YYYYMMDDHHMM. The version is MARKETING_VERSION in
# ios/project.yml. Output goes to $TARGET/ios: Cahoots.xcarchive and export/Cahoots.ipa.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
UPLOAD=1
[ "${1:-}" = "--no-upload" ] && UPLOAD=0

[ -f ios/Signing.xcconfig ] || { echo "Copy ios/Signing.xcconfig.example to ios/Signing.xcconfig and put your team id in it."; exit 1; }
TEAM=$(sed -nE 's/^[[:space:]]*DEVELOPMENT_TEAM[[:space:]]*=[[:space:]]*([A-Z0-9]+).*/\1/p' ios/Signing.xcconfig | head -1)
if [ -z "$TEAM" ] || [ "$TEAM" = "YOUR_TEAM_ID" ]; then
  echo "ios/Signing.xcconfig has no team id."
  exit 1
fi
BUILD=$(date -u +%Y%m%d%H%M)
OUT="$TARGET/ios"

scripts/ios-framework.sh >/dev/null
(cd ios && xcodegen generate --quiet)

echo "archiving build $BUILD for team $TEAM"
rm -rf "$OUT/Cahoots.xcarchive" "$OUT/export"
xcodebuild archive -project ios/Cahoots.xcodeproj -scheme Cahoots -configuration Release \
  -destination 'generic/platform=iOS' -archivePath "$OUT/Cahoots.xcarchive" \
  -allowProvisioningUpdates DEVELOPMENT_TEAM="$TEAM" CURRENT_PROJECT_VERSION="$BUILD" \
  >"$OUT/archive.log" 2>&1 || { grep -E "error:|error -" "$OUT/archive.log" | sort -u; echo "archive failed, log: $OUT/archive.log"; exit 1; }

OPTIONS=$(mktemp -t ExportOptions).plist
sed "s/TEAM_ID/$TEAM/" ios/ExportOptions.plist >"$OPTIONS"
xcodebuild -exportArchive -archivePath "$OUT/Cahoots.xcarchive" -exportOptionsPlist "$OPTIONS" \
  -exportPath "$OUT/export" -allowProvisioningUpdates \
  >"$OUT/export.log" 2>&1 || { grep -E "error:|error -" "$OUT/export.log" | sort -u; echo "export failed, log: $OUT/export.log"; exit 1; }
rm -f "$OPTIONS"
IPA="$OUT/export/Cahoots.ipa"
echo "exported $IPA (build $BUILD)"

if [ "$UPLOAD" = 0 ]; then
  exit 0
fi
if [ -z "${ASC_KEY_ID:-}" ] || [ -z "${ASC_ISSUER_ID:-}" ] \
   || [ ! -f "$HOME/.appstoreconnect/private_keys/AuthKey_${ASC_KEY_ID:-x}.p8" ]; then
  echo "Not uploaded. Set ASC_KEY_ID and ASC_ISSUER_ID and put the key at"
  echo "~/.appstoreconnect/private_keys/AuthKey_<ASC_KEY_ID>.p8, then run this again,"
  echo "or upload $IPA with Transporter."
  exit 0
fi
xcrun altool --upload-app --type ios -f "$IPA" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
echo "uploaded build $BUILD. It appears in TestFlight once App Store Connect has processed it."
