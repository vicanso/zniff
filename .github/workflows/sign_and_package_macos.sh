#!/usr/bin/env bash
set -euo pipefail

# Shared macOS codesign, embedded NETransparentProxyProvider System Extension
# builder/signer, DMG creation, Apple notarization, and stapler helper for
# zniff (`zniff.app`).
#
# Subcommands:
#   --preflight     Validate signing & notary environment variables and Apple credentials
#   --embed-sysext  Compile, embed, and inside-out sign dev.zniff.app.transparent-proxy.systemextension
#   --package       Verify universal .app, embed/sign, build & sign DMG, notarize, staple, stage assets
#   --cleanup       Remove temporary certificate, provisioning profiles, and keychain

mode="${1:-}"
if [ -z "$mode" ]; then
  echo "Usage: $0 <--preflight|--embed-sysext|--package|--cleanup>" >&2
  exit 1
fi

require_env() {
  local name="$1"
  if [ -z "${!name:-}" ]; then
    echo "::error::Missing required environment variable: ${name}" >&2
    exit 1
  fi
}

codesign_with_retry() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if codesign "$@"; then
      echo "codesign succeeded on attempt ${attempt}"
      return 0
    fi
    if [ "$attempt" -eq 5 ]; then
      echo "::error::codesign still failing after ${attempt} attempts" >&2
      return 1
    fi
    echo "codesign attempt ${attempt} failed (timestamp server?); retrying in 15s..."
    sleep 15
  done
}

staple_with_retry() {
  local target="$1"
  local attempt
  for attempt in 1 2 3; do
    if xcrun stapler staple "$target"; then
      echo "stapled ${target} on attempt ${attempt}"
      return 0
    fi
    if [ "$attempt" -eq 3 ]; then
      echo "::error::xcrun stapler staple failed for ${target} after ${attempt} attempts" >&2
      return 1
    fi
    echo "stapler attempt ${attempt} for ${target} failed (propagation?); retrying in 15s..."
    sleep 15
  done
}

preflight() {
  require_env MACOS_CERTIFICATE
  require_env MACOS_CERTIFICATE_PWD
  require_env AC_USERNAME
  require_env AC_PASSWORD
  require_env AC_TEAM_ID

  xcrun notarytool history \
    --apple-id "$AC_USERNAME" \
    --password "$AC_PASSWORD" \
    --team-id "$AC_TEAM_ID" >/dev/null
  echo "Notary credentials OK"
}

cleanup() {
  local tmp_root="${RUNNER_TEMP:-/tmp}"
  local keychain_path="${KEYCHAIN_PATH:-${tmp_root}/app-signing.keychain-db}"
  rm -f \
    "${tmp_root}/certificate.p12" \
    "${tmp_root}/app.provisionprofile" \
    "${tmp_root}/sysext.provisionprofile"
  security delete-keychain "$keychain_path" >/dev/null 2>&1 || rm -f "$keychain_path"
}

embed_sysext() {
  require_env APP_PATH
  require_env ZNIFF_APP_PROFILE
  require_env ZNIFF_SYSEXT_PROFILE

  if [ ! -d "$APP_PATH" ]; then
    echo "::error::Missing .app bundle at ${APP_PATH}" >&2
    exit 1
  fi
  if [ ! -f "$ZNIFF_APP_PROFILE" ]; then
    echo "::error::Missing ZNIFF_APP_PROFILE at ${ZNIFF_APP_PROFILE}" >&2
    exit 1
  fi
  if [ ! -f "$ZNIFF_SYSEXT_PROFILE" ]; then
    echo "::error::Missing ZNIFF_SYSEXT_PROFILE at ${ZNIFF_SYSEXT_PROFILE}" >&2
    exit 1
  fi

  local script_dir repo_root tp_dir
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "${script_dir}/../.." && pwd)"
  tp_dir="${repo_root}/macos/transparent-proxy"

  local app_basename bin_path archs
  app_basename="$(basename "$APP_PATH")"
  bin_path="${APP_PATH}/Contents/MacOS/${app_basename%.app}"
  if [ ! -f "$bin_path" ]; then
    bin_path="${APP_PATH}/Contents/MacOS/zniff"
  fi
  if [ ! -f "$bin_path" ]; then
    echo "::error::Missing host executable inside ${APP_PATH}/Contents/MacOS" >&2
    exit 1
  fi

  archs="$(lipo -archs "$bin_path" 2>/dev/null || true)"
  if [ -z "$archs" ]; then
    archs="$(uname -m)"
  fi

  local short_version build_version cargo_version
  cargo_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${repo_root}/app/Cargo.toml" | head -1)"
  short_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "${APP_PATH}/Contents/Info.plist" 2>/dev/null || true)"
  build_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "${APP_PATH}/Contents/Info.plist" 2>/dev/null || true)"
  short_version="${short_version:-${cargo_version}}"
  build_version="${build_version:-${short_version}}"

  local team_id
  team_id="$(python3 -c '
import plistlib, subprocess, sys
raw = subprocess.check_output(["security", "cms", "-D", "-i", sys.argv[1]], stderr=subprocess.DEVNULL)
pl = plistlib.loads(raw)
tid = pl.get("Entitlements", {}).get("com.apple.developer.team-identifier", "")
if not tid:
    tids = pl.get("TeamIdentifier", [])
    tid = tids[0] if tids else ""
print(tid.strip())
' "$ZNIFF_SYSEXT_PROFILE" 2>/dev/null || true)"
  case "$team_id" in
    [A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9]) ;;
    *) team_id="4G5PXJXZTU" ;;
  esac

  local sysext_bundle sysext_bin tmp_work
  sysext_bundle="${APP_PATH}/Contents/Library/SystemExtensions/dev.zniff.app.transparent-proxy.systemextension"
  sysext_bin="${sysext_bundle}/Contents/MacOS/ZniffTransparentProxy"
  mkdir -p "${sysext_bundle}/Contents/MacOS"

  tmp_work="$(mktemp -d)"
  trap 'rm -rf "'"$tmp_work"'"' RETURN

  local arch_bins=()
  local arch swift_target arch_out
  for arch in $archs; do
    case "$arch" in
      arm64) swift_target="arm64-apple-macos12.0" ;;
      x86_64) swift_target="x86_64-apple-macos12.0" ;;
      *)
        echo "::error::Unsupported macOS arch for sysext: ${arch}" >&2
        exit 1
        ;;
    esac
    arch_out="${tmp_work}/ZniffTransparentProxy-${arch}"
    xcrun swiftc -O -swift-version 5 -target "$swift_target" \
      -module-name ZniffTransparentProxy \
      -framework NetworkExtension \
      -framework Network \
      -framework Foundation \
      "${tp_dir}/main.swift" \
      -o "$arch_out"
    arch_bins+=("$arch_out")
  done

  if [ "${#arch_bins[@]}" -gt 1 ]; then
    lipo -create "${arch_bins[@]}" -output "$sysext_bin"
  else
    cp "${arch_bins[0]}" "$sysext_bin"
  fi
  chmod 755 "$sysext_bin"
  echo "Built sysext binary architectures: $(lipo -archs "$sysext_bin")"

  sed \
    -e "s/__VERSION__/${short_version}/g" \
    -e "s/__BUILD_VERSION__/${build_version}/g" \
    -e "s/__TEAM_ID__/${team_id}/g" \
    "${tp_dir}/Info.plist.template" > "${sysext_bundle}/Contents/Info.plist"
  plutil -lint "${sysext_bundle}/Contents/Info.plist"

  cp "$ZNIFF_SYSEXT_PROFILE" "${sysext_bundle}/Contents/embedded.provisionprofile"
  cp "$ZNIFF_APP_PROFILE" "${APP_PATH}/Contents/embedded.provisionprofile"

  local ent_app ent_sysext
  ent_app="${tmp_work}/entitlements-app.plist"
  ent_sysext="${tmp_work}/entitlements-sysext.plist"
  sed "s/__TEAM_ID__/${team_id}/g" "${tp_dir}/entitlements-app.plist" > "$ent_app"
  sed "s/__TEAM_ID__/${team_id}/g" "${tp_dir}/entitlements-sysext.plist" > "$ent_sysext"
  plutil -lint "$ent_app" "$ent_sysext"

  local sign_identity="${ZNIFF_SIGN_IDENTITY:-}"
  local keychain_args=()
  if [ -n "${KEYCHAIN_PATH:-}" ]; then
    keychain_args=(--keychain "$KEYCHAIN_PATH")
  fi

  xattr -cr "$APP_PATH"
  if [ -n "$sign_identity" ]; then
    # Sign inside-out without `--deep` so host and sysext keep distinct entitlements.
    codesign_with_retry \
      --force \
      --verbose \
      --options runtime \
      --timestamp \
      ${keychain_args[@]+"${keychain_args[@]}"} \
      --entitlements "$ent_sysext" \
      --sign "$sign_identity" \
      "$sysext_bundle"
    codesign_with_retry \
      --force \
      --verbose \
      --options runtime \
      --timestamp \
      ${keychain_args[@]+"${keychain_args[@]}"} \
      --entitlements "$ent_app" \
      --sign "$sign_identity" \
      "$APP_PATH"
  else
    codesign -s - --force --entitlements "$ent_sysext" "$sysext_bundle"
    codesign -s - --force --entitlements "$ent_app" "$APP_PATH"
  fi

  codesign --verify --deep --strict --verbose=2 "$APP_PATH"
}

package_macos() {
  require_env APP_PATH
  require_env STAGE_DIR
  require_env DMG_NAME
  require_env TAR_NAME
  require_env MACOS_CERTIFICATE
  require_env MACOS_CERTIFICATE_PWD
  require_env AC_USERNAME
  require_env AC_PASSWORD
  require_env AC_TEAM_ID

  local vol_name="${VOL_NAME:-zniff Installer}"
  local tmp_root="${RUNNER_TEMP:-/tmp}"
  local keychain_path="${KEYCHAIN_PATH:-${tmp_root}/app-signing.keychain-db}"
  local cert_path="${tmp_root}/certificate.p12"

  trap 'rm -f "'"$cert_path"'"' EXIT

  if [ ! -d "$APP_PATH" ]; then
    echo "::error::Missing .app bundle at ${APP_PATH}" >&2
    exit 1
  fi

  local app_basename bin_path archs
  app_basename="$(basename "$APP_PATH")"
  bin_path="${APP_PATH}/Contents/MacOS/${app_basename%.app}"
  if [ ! -f "$bin_path" ]; then
    echo "::error::Missing app executable at ${bin_path}" >&2
    exit 1
  fi
  archs="$(lipo -archs "$bin_path")"
  case "$archs" in
    *arm64*x86_64*|*x86_64*arm64*)
      echo "Verified universal binary architectures: ${archs}"
      ;;
    *)
      echo "::error::Expected universal arm64+x86_64 binary, got: ${archs}" >&2
      exit 1
      ;;
  esac

  # Create an isolated temporary keychain under RUNNER_TEMP (never inside checkout).
  security delete-keychain "$keychain_path" >/dev/null 2>&1 || rm -f "$keychain_path"
  security create-keychain -p "" "$keychain_path"
  security set-keychain-settings -lut 21600 "$keychain_path"
  security unlock-keychain -p "" "$keychain_path"

  printf '%s' "$MACOS_CERTIFICATE" | base64 --decode > "$cert_path"
  security import "$cert_path" \
    -k "$keychain_path" \
    -P "$MACOS_CERTIFICATE_PWD" \
    -T /usr/bin/codesign \
    -T /usr/bin/security
  rm -f "$cert_path"

  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "" "$keychain_path"
  security list-keychains -d user -s "$keychain_path"

  local cert_id="${MACOS_CERTIFICATE_NAME:-}"
  if [ -z "$cert_id" ]; then
    cert_id="$(security find-identity -v -p codesigning "$keychain_path" | awk '/"Developer ID Application:/ { print $2; exit }')"
  fi
  if [ -z "$cert_id" ]; then
    echo "::error::No Developer ID Application signing identity found in temporary keychain" >&2
    security find-identity -v -p codesigning "$keychain_path" >&2 || true
    exit 1
  fi
  echo "Signing identity: $(security find-identity -v -p codesigning "$keychain_path" | grep -F "$cert_id" | head -n 1 || echo "$cert_id")"

  xattr -cr "$APP_PATH"

  local app_prof_b64="${MACOS_APP_PROVISIONING_PROFILE:-}"
  local sysext_prof_b64="${MACOS_SYSEXT_PROVISIONING_PROFILE:-}"
  if [ -n "$app_prof_b64" ] && [ -n "$sysext_prof_b64" ]; then
    local app_prof="${tmp_root}/app.provisionprofile"
    local sysext_prof="${tmp_root}/sysext.provisionprofile"
    printf '%s' "$app_prof_b64" | base64 --decode > "$app_prof"
    printf '%s' "$sysext_prof_b64" | base64 --decode > "$sysext_prof"

    APP_PATH="$APP_PATH" \
    ZNIFF_APP_PROFILE="$app_prof" \
    ZNIFF_SYSEXT_PROFILE="$sysext_prof" \
    ZNIFF_SIGN_IDENTITY="$cert_id" \
    KEYCHAIN_PATH="$keychain_path" \
      embed_sysext

    local sysext_bundle sysext_bin sysext_archs
    sysext_bundle="${APP_PATH}/Contents/Library/SystemExtensions/dev.zniff.app.transparent-proxy.systemextension"
    sysext_bin="${sysext_bundle}/Contents/MacOS/ZniffTransparentProxy"
    sysext_archs="$(lipo -archs "$sysext_bin")"
    case "$sysext_archs" in
      *arm64*x86_64*|*x86_64*arm64*)
        echo "Verified universal sysext architectures: ${sysext_archs}"
        ;;
      *)
        echo "::error::Expected universal arm64+x86_64 sysext binary, got: ${sysext_archs}" >&2
        exit 1
        ;;
    esac
    if ! codesign -dvv "$sysext_bundle" 2>&1 | grep -q "^Timestamp="; then
      echo "::error::No secure timestamp embedded in signed ${sysext_bundle}" >&2
      exit 1
    fi
  else
    codesign_with_retry \
      --force \
      --deep \
      --verbose \
      --options runtime \
      --timestamp \
      --keychain "$keychain_path" \
      --sign "$cert_id" \
      "$APP_PATH"
    codesign --verify --deep --strict --verbose=2 "$APP_PATH"
  fi

  if ! codesign -dvv "$APP_PATH" 2>&1 | grep -q "^Timestamp="; then
    echo "::error::No secure timestamp embedded in signed ${APP_PATH}" >&2
    exit 1
  fi

  mkdir -p "$STAGE_DIR"
  local dmg_path="${STAGE_DIR}/${DMG_NAME}"
  local tar_path="${STAGE_DIR}/${TAR_NAME}"
  rm -f "$dmg_path" "$tar_path"

  create-dmg \
    --volname "$vol_name" \
    --window-pos 200 120 \
    --window-size 800 400 \
    --icon-size 100 \
    --icon "$app_basename" 200 190 \
    --hide-extension "$app_basename" \
    --app-drop-link 600 185 \
    "$dmg_path" \
    "$APP_PATH"

  codesign_with_retry \
    --force \
    --verbose \
    --timestamp \
    --keychain "$keychain_path" \
    --sign "$cert_id" \
    "$dmg_path"

  echo "Submitting ${dmg_path} for Apple notarization..."
  xcrun notarytool submit "$dmg_path" \
    --apple-id "$AC_USERNAME" \
    --password "$AC_PASSWORD" \
    --team-id "$AC_TEAM_ID" \
    --wait

  sleep 30
  staple_with_retry "$dmg_path"
  staple_with_retry "$APP_PATH"

  xcrun stapler validate "$dmg_path"
  xcrun stapler validate "$APP_PATH"
  spctl -a -t open --context context:primary-signature -v "$dmg_path"

  # Repackage the signed and stapled .app for archive downloads.
  COPYFILE_DISABLE=1 tar -czf "$tar_path" -C "$(dirname "$APP_PATH")" "$app_basename"
  ls -lh "$dmg_path" "$tar_path"
}

case "$mode" in
  --preflight) preflight ;;
  --embed-sysext) embed_sysext ;;
  --package) package_macos ;;
  --cleanup) cleanup ;;
  *)
    echo "Unknown mode: ${mode}" >&2
    exit 1
    ;;
esac
