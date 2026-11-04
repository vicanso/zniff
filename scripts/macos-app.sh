#!/bin/sh
# Wrap a zniff binary in a macOS .app that uses icons/zniff.icns.
# Usage: scripts/macos-app.sh <binary> <dest.app>
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
  echo "macos-app.sh only runs on macOS" >&2
  exit 1
fi

bin=$1
dest=$2
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
icon=$root/icons/zniff.icns
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/app/Cargo.toml" | head -1)

if [ ! -f "$bin" ]; then
  echo "binary not found: $bin" >&2
  exit 1
fi
if [ ! -f "$icon" ]; then
  echo "icon not found: $icon" >&2
  exit 1
fi
if [ -z "$version" ]; then
  echo "version missing from app/Cargo.toml" >&2
  exit 1
fi

rm -rf "$dest"
mkdir -p "$dest/Contents/MacOS" "$dest/Contents/Resources"
cp "$bin" "$dest/Contents/MacOS/zniff"
chmod 755 "$dest/Contents/MacOS/zniff"
cp "$icon" "$dest/Contents/Resources/zniff.icns"

cat > "$dest/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>zniff</string>
  <key>CFBundleDisplayName</key>
  <string>zniff</string>
  <key>CFBundleIdentifier</key>
  <string>dev.zniff.app</string>
  <key>CFBundleVersion</key>
  <string>${version}</string>
  <key>CFBundleShortVersionString</key>
  <string>${version}</string>
  <key>CFBundleExecutable</key>
  <string>zniff</string>
  <key>CFBundleIconFile</key>
  <string>zniff</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

# Ad-hoc, so the bundle is a signed app on this machine. A Developer ID
# signature, when there is one, replaces this.
codesign -s - --force --deep "$dest"
echo "$dest"
