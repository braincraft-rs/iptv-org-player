#!/usr/bin/env bash
# Build a self-contained macOS .app (libmpv and its dylibs bundled) and zip it.
# Needs: cargo, Homebrew mpv, dylibbundler (brew install mpv dylibbundler).
set -euo pipefail
cd "$(dirname "$0")/.."

NAME="IPTV-Org Player"
BIN="iptv-org-player"
VERSION="$(cargo pkgid | sed 's/.*[#@]//')"
ARCH="$(uname -m)"
APP="dist/${NAME}.app"
ZIP="dist/IPTV-Org-Player-${VERSION}-macos-${ARCH}.zip"

cargo build --release
rm -rf "$APP" "$ZIP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"
cp "target/release/${BIN}" "$APP/Contents/MacOS/${BIN}"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>${NAME}</string>
  <key>CFBundleDisplayName</key><string>${NAME}</string>
  <key>CFBundleIdentifier</key><string>rs.braincraft.iptv-org-player</string>
  <key>CFBundleExecutable</key><string>${BIN}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Copy libmpv + every dylib it needs into Contents/Frameworks and rewrite the
# load paths so the app does not depend on Homebrew being installed.
dylibbundler -od -b \
  -x "$APP/Contents/MacOS/${BIN}" \
  -d "$APP/Contents/Frameworks" \
  -p "@executable_path/../Frameworks/"

# dylibbundler can leave the same LC_RPATH twice in a library, which dyld
# rejects ("duplicate LC_RPATH"). Remove the extra copy.
for f in "$APP"/Contents/Frameworks/*.dylib "$APP/Contents/MacOS/${BIN}"; do
  count=$(otool -l "$f" | grep -c "path @executable_path/../Frameworks/ " || true)
  while [ "$count" -gt 1 ]; do
    install_name_tool -delete_rpath "@executable_path/../Frameworks/" "$f"
    count=$((count - 1))
  done
done

# Apple Silicon refuses unsigned code; an ad-hoc signature is enough to run.
codesign --force --deep --sign - "$APP"

ditto -c -k --keepParent "$APP" "$ZIP"
echo "Built: $APP"
echo "Zip:   $ZIP ($(du -h "$ZIP" | cut -f1))"
