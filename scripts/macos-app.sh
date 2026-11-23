#!/bin/sh
# Wrap a zniff binary in a macOS .app that uses icons/zniff.icns.
# Usage: scripts/macos-app.sh <binary> <dest.app>
# Optional env for embedded NETransparentProxyProvider System Extension:
#   ZNIFF_APP_PROFILE    path to host app Developer ID .provisionprofile
#   ZNIFF_SYSEXT_PROFILE path to sysext Developer ID .provisionprofile
#   ZNIFF_SIGN_IDENTITY  codesign identity (name or SHA-1); if omitted, ad-hoc signs
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

sign_with_retry() {
  target_path=$1
  entitlements_path=$2
  identity=$3
  for attempt in 1 2 3 4 5; do
    if codesign --force --verbose --options runtime --timestamp \
      --entitlements "$entitlements_path" \
      --sign "$identity" \
      "$target_path"; then
      return 0
    fi
    if [ "$attempt" -eq 5 ]; then
      echo "codesign still failing after $attempt attempts for $target_path" >&2
      return 1
    fi
    echo "codesign attempt $attempt failed for $target_path; retrying in 15s…" >&2
    sleep 15
  done
}

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

app_profile=${ZNIFF_APP_PROFILE:-}
sysext_profile=${ZNIFF_SYSEXT_PROFILE:-}
sign_identity=${ZNIFF_SIGN_IDENTITY:-}

if [ -n "$app_profile" ] && [ -n "$sysext_profile" ]; then
  if [ ! -f "$app_profile" ]; then
    echo "ZNIFF_APP_PROFILE not found: $app_profile" >&2
    exit 1
  fi
  if [ ! -f "$sysext_profile" ]; then
    echo "ZNIFF_SYSEXT_PROFILE not found: $sysext_profile" >&2
    exit 1
  fi

  tp_dir="$root/macos/transparent-proxy"
  arch=$(lipo -archs "$bin" 2>/dev/null | awk '{print $1}')
  if [ -z "$arch" ]; then
    arch=$(uname -m)
  fi
  case "$arch" in
    arm64) swift_target="arm64-apple-macos12.0" ;;
    x86_64) swift_target="x86_64-apple-macos12.0" ;;
    *)
      echo "unsupported macOS arch for sysext: $arch" >&2
      exit 1
      ;;
  esac

  team_id=$(python3 -c '
import plistlib, subprocess, sys
raw = subprocess.check_output(["security", "cms", "-D", "-i", sys.argv[1]], stderr=subprocess.DEVNULL)
pl = plistlib.loads(raw)
tid = pl.get("Entitlements", {}).get("com.apple.developer.team-identifier", "")
if not tid:
    tids = pl.get("TeamIdentifier", [])
    tid = tids[0] if tids else ""
print(tid.strip())
' "$sysext_profile" 2>/dev/null || true)
  case "$team_id" in
    [A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9]) ;;
    *) team_id="4G5PXJXZTU" ;;
  esac

  sysext_bundle="$dest/Contents/Library/SystemExtensions/dev.zniff.app.transparent-proxy.systemextension"
  mkdir -p "$sysext_bundle/Contents/MacOS"

  xcrun swiftc -O -swift-version 5 -target "$swift_target" \
    -module-name ZniffTransparentProxy \
    -framework NetworkExtension \
    -framework Network \
    -framework Foundation \
    "$tp_dir/main.swift" \
    -o "$sysext_bundle/Contents/MacOS/ZniffTransparentProxy"
  chmod 755 "$sysext_bundle/Contents/MacOS/ZniffTransparentProxy"

  sed -e "s/__VERSION__/${version}/g" -e "s/__BUILD_VERSION__/${version}/g" -e "s/__TEAM_ID__/${team_id}/g" \
    "$tp_dir/Info.plist.template" > "$sysext_bundle/Contents/Info.plist"
  plutil -lint "$sysext_bundle/Contents/Info.plist"

  cp "$sysext_profile" "$sysext_bundle/Contents/embedded.provisionprofile"
  cp "$app_profile" "$dest/Contents/embedded.provisionprofile"

  ent_tmp_dir=$(mktemp -d)
  trap 'rm -rf "$ent_tmp_dir"' EXIT INT TERM
  ent_app="$ent_tmp_dir/entitlements-app.plist"
  ent_sysext="$ent_tmp_dir/entitlements-sysext.plist"
  sed "s/__TEAM_ID__/${team_id}/g" "$tp_dir/entitlements-app.plist" > "$ent_app"
  sed "s/__TEAM_ID__/${team_id}/g" "$tp_dir/entitlements-sysext.plist" > "$ent_sysext"
  plutil -lint "$ent_app" "$ent_sysext"

  if [ -n "$sign_identity" ]; then
    sign_with_retry "$sysext_bundle" "$ent_sysext" "$sign_identity"
    sign_with_retry "$dest" "$ent_app" "$sign_identity"
  else
    codesign -s - --force --entitlements "$ent_sysext" "$sysext_bundle"
    codesign -s - --force --entitlements "$ent_app" "$dest"
  fi
elif [ -n "$sign_identity" ]; then
  codesign --force --deep --verbose --options runtime --timestamp --sign "$sign_identity" "$dest"
else
  # Ad-hoc, so the bundle is a signed app on this machine. A Developer ID
  # signature, when there is one, replaces this.
  codesign -s - --force --deep "$dest"
fi

echo "$dest"
