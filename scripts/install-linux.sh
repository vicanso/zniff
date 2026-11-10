#!/usr/bin/env sh
#
# User-level install: binary, icon, launcher entry, autostart.
# No root, nothing outside $HOME.
#
# Linux only, and the name says so — but the runtime guard below stays,
# because a file name cannot stop anyone running this on macOS. There a
# bare binary is not an install at all: the release is a signed .app
# inside the DMG, and none of the XDG paths this writes to mean anything.
#
# Three sources, tried in that order, because the same file ships inside
# the release tarball and also stands on its own:
#
#   1. a binary next to this script   — unpacked tarball; no network
#   2. the repo's app/target/release  — `make release` (the crate is app/)
#   3. the latest release, downloaded — this script on its own
#
# The tarball member is `zniff-linux-$arch/zniff`, the same path
# `updater.rs` replaces in place. `--uninstall` removes exactly what this
# put down. `--no-autostart` installs everything else. `--tag vX.Y.Z`
# pins the download.
#
# Not meant to be piped into a shell. It stops a running process and
# writes under ~/.local and ~/.config; that is not something to run
# sight-unseen out of a pipe. Installing does not turn the system proxy on.

set -eu

APP=zniff
DESKTOP_ID=dev.zniff.app
REPO=vicanso/zniff
# Both hosts carry the same files under the same names, and their
# download URLs differ only in the root — the same pair `updater.rs`
# uses. GitHub first because it is the origin; Gitee second because for
# the networks that cannot reach GitHub at all it is the only one, and
# that is the entire reason the mirror exists.
GITHUB_API="https://api.github.com/repos/$REPO"
GITHUB_DL="https://github.com/$REPO/releases/download"
GITEE_API="https://gitee.com/api/v5/repos/$REPO"
GITEE_DL="https://gitee.com/$REPO/releases/download"

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/$APP"
DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/512x512/apps"
AUTOSTART_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"

say() { printf '%s\n' "$*"; }
die() { printf 'install-linux.sh: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || die "Linux only — macOS installs the signed .app from the release page."

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

uninstall() {
    for f in "$BIN_DIR/$APP" "$DESKTOP_DIR/$DESKTOP_ID.desktop" \
             "$ICON_DIR/$APP.png" "$AUTOSTART_DIR/$DESKTOP_ID.desktop"; do
        if [ -e "$f" ]; then
            rm -f "$f"
            say "removed $f"
        fi
    done
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
    fi
    # The generator keeps serving a unit for a file that is gone until it
    # is told otherwise.
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload 2>/dev/null || true
    fi
    say ""
    say "Left alone on purpose: $DATA_DIR (config, root CA)."
    say "Remove it by hand if you want that gone too."
    exit 0
}

if [ "${1:-}" = --uninstall ]; then
    uninstall
fi

want_autostart=1
pin_tag=""
while [ $# -gt 0 ]; do
    case "$1" in
        --no-autostart) want_autostart=0 ;;
        --tag) shift; pin_tag="${1:-}"; [ -n "$pin_tag" ] || die "--tag needs a version" ;;
        *) die "unknown argument $1 (--uninstall, --no-autostart, --tag vX.Y.Z)" ;;
    esac
    shift
done

# --- what to install ------------------------------------------------------

tmpdir=""
cleanup() {
    if [ -n "$tmpdir" ]; then rm -rf "$tmpdir"; fi
}
trap cleanup EXIT INT TERM

case "$(uname -m)" in
    x86_64|amd64)   arch=x86_64 ;;
    aarch64|arm64)  arch=aarch64 ;;
    *) arch="" ;;
esac
tarball="$APP-linux-$arch.tar.gz"

latest_tag() {
    # One `tag_name` at the top level of `releases/latest` on both hosts.
    # Split on commas first so the match cannot run past its own field.
    curl -fsSL --max-time 20 "$1/releases/latest" 2>/dev/null |
        tr ',' '\n' | grep '"tag_name"' | head -1 |
        sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/'
}

# Fetch the tarball into a temp dir and verify it. Sets `unpacked` on
# success. Verifying is not optional: the whole reason this app can have
# a mirror at all is that both hosts serve the *same bytes* and
# SHA256SUMS says which — a second address, never a second build. An
# installer that skipped the check would make the mirror a second trust
# assumption. A checksum mismatch stops here; it is not a reason to try
# the other host.
fetch_release() {
    [ -n "$arch" ] || die "unsupported architecture $(uname -m) — build from source instead"
    command -v curl >/dev/null 2>&1 || die "curl is needed to download a release"
    command -v sha256sum >/dev/null 2>&1 || die "sha256sum is needed to verify the download"

    tmpdir=$(mktemp -d)
    for host in github gitee; do
        if [ "$host" = github ]; then api="$GITHUB_API"; dl="$GITHUB_DL"; else api="$GITEE_API"; dl="$GITEE_DL"; fi

        tag="$pin_tag"
        if [ -z "$tag" ]; then tag=$(latest_tag "$api") || tag=""; fi
        if [ -z "$tag" ]; then
            say "  $host: no release found"
            continue
        fi

        say "  $host: $tag"
        if ! curl -fsSL --max-time 300 -o "$tmpdir/$tarball" "$dl/$tag/$tarball" 2>/dev/null; then
            say "  $host: $tag carries no $tarball"
            continue
        fi
        if ! curl -fsSL --max-time 60 -o "$tmpdir/SHA256SUMS" "$dl/$tag/SHA256SUMS" 2>/dev/null; then
            say "  $host: $tag has no SHA256SUMS — refusing to install unverified bytes"
            continue
        fi
        if ! grep -F " $tarball" "$tmpdir/SHA256SUMS" > "$tmpdir/$tarball.sha256"; then
            say "  $host: SHA256SUMS does not list $tarball — refusing to install unverified bytes"
            continue
        fi
        if ! (cd "$tmpdir" && sha256sum -c "$tarball.sha256" >/dev/null 2>&1); then
            die "checksum mismatch on $tarball from $host — not installing"
        fi
        say "  $host: $tarball verified against SHA256SUMS"

        (cd "$tmpdir" && tar -xzf "$tarball")
        unpacked="$tmpdir/$APP-linux-$arch"
        [ -x "$unpacked/$APP" ] || die "the tarball has no $APP binary in it"
        return 0
    done
    die "no Linux release available from either host (built one locally? run 'make release' first)"
}

if [ -x "$here/$APP" ]; then
    binary="$here/$APP"
    icon="$here/$APP.png"
elif [ -x "$here/../app/target/release/$APP" ]; then
    binary="$here/../app/target/release/$APP"
    icon="$here/../icons/$APP.png"
elif [ -x "$here/../target/release/$APP" ]; then
    binary="$here/../target/release/$APP"
    icon="$here/../icons/$APP.png"
else
    say "no local binary — fetching the latest release for $arch"
    fetch_release
    binary="$unpacked/$APP"
    icon="$unpacked/$APP.png"
fi
[ -f "$icon" ] || icon=""

# --- a running instance would keep serving the old binary -----------------
#
# The process keeps the old text mapped and the listen port open, so a
# fresh binary on disk changes nothing until that process is gone. Stop
# it here. SIGTERM does not run the normal exit path; if this process had
# rewritten the system proxy, the next launch restores it from
# proxy-snapshot.toml.
if pgrep -x "$APP" >/dev/null 2>&1; then
    say "stopping the running $APP (it would keep serving the old binary)"
    pkill -x "$APP" || true
    sleep 1
fi

# --- install --------------------------------------------------------------

mkdir -p "$BIN_DIR"
install -m755 "$binary" "$BIN_DIR/$APP"
say "installed $BIN_DIR/$APP"

icon_installed=""
if [ -n "$icon" ]; then
    mkdir -p "$ICON_DIR"
    install -m644 "$icon" "$ICON_DIR/$APP.png"
    icon_installed=1
    say "installed $ICON_DIR/$APP.png"
fi

# --- shared libraries, the one thing that really is per-distribution ------
#
# Everything above is XDG and identical on every distribution. Library
# *package names* are not. This binary links GTK 4 at build time (4.14 or
# later — the interface is nsh's GTK backend). It is an ordinary window,
# not a layer-shell panel, so gtk4-layer-shell is not required. Without
# GTK the first launch dies in the dynamic linker, which says nothing a
# user can act on.
#
# The release is built on Ubuntu 24.04, so it also needs glibc 2.39.
# Older distributions fail that check even when a GTK package exists.
#
# Reported, never installed. A user-level installer that starts calling a
# package manager needs root it was designed not to need, and a guess at
# the wrong manager is worse than a sentence the reader can act on
# themselves.
distro_packages() {
    id=""
    like=""
    if [ -r /etc/os-release ]; then
        id=$(sed -n 's/^ID=//p' /etc/os-release | tr -d '"' | head -1)
        like=$(sed -n 's/^ID_LIKE=//p' /etc/os-release | tr -d '"' | head -1)
    fi
    case " $id $like " in
        *" arch "*|*" archlinux "*)
            echo "sudo pacman -S --needed gtk4" ;;
        *" debian "*|*" ubuntu "*)
            echo "sudo apt install libgtk-4-1" ;;
        *" fedora "*|*" rhel "*|*" centos "*)
            echo "sudo dnf install gtk4" ;;
        *" suse "*|*" opensuse "*)
            echo "sudo zypper install libgtk-4-1" ;;
        *)
            echo "" ;;
    esac
}

report_missing_libs() {
    command -v ldd >/dev/null 2>&1 || return 0
    report=$(ldd "$1" 2>&1) || true
    missing=$(printf '%s\n' "$report" | awk '/not found/ { print $1 }' | sort -u)
    glibc=$(printf '%s\n' "$report" | grep 'GLIBC_' || true)
    if [ -n "$glibc" ]; then
        say ""
        say "warning: this $APP build needs glibc 2.39 (the Ubuntu 24.04 baseline)."
        say "This system is older. Build from source instead of using the release binary."
        printf '%s\n' "$glibc" | sed 's/^/  /'
    fi
    [ -n "$missing" ] || return 0
    say ""
    say "warning: $APP cannot start yet — these shared libraries are missing:"
    printf '%s\n' "$missing" | sed 's/^/  /'
    pkgs=$(distro_packages)
    if [ -n "$pkgs" ]; then
        say "Install them with:"
        say "  $pkgs"
        say "GTK 4.14 or later is required."
    else
        say "Install your distribution's GTK 4 (4.14 or later)."
    fi
}

report_missing_libs "$BIN_DIR/$APP"

# One launcher entry, and optionally the same file under autostart.
# StartupNotify stays on: zniff maps a normal toplevel, so the session
# has a window to settle the startup cursor against. The desktop id
# matches the GTK application id (`dev.zniff.app`).
write_entry() {
    icon_key=""
    if [ -n "$icon_installed" ]; then
        icon_key="Icon=$APP"
    fi
    cat > "$1" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=$APP
GenericName=HTTP capture proxy
GenericName[zh_CN]=HTTP 抓包代理
Comment=Desktop HTTP(S) capture and debugging proxy
Comment[zh_CN]=桌面 HTTP(S) 抓包与调试代理
Exec="$BIN_DIR/$APP"
$icon_key
Terminal=false
StartupNotify=true
StartupWMClass=$DESKTOP_ID
Categories=Network;
Keywords=proxy;http;https;capture;debug;
EOF
}

mkdir -p "$DESKTOP_DIR"
write_entry "$DESKTOP_DIR/$DESKTOP_ID.desktop"
say "installed $DESKTOP_DIR/$DESKTOP_ID.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi

# --- autostart, only where something actually reads it ---------------------
#
# A compositor is not a session manager. Hyprland, Sway, river and niri do
# not read ~/.config/autostart themselves; on a systemd distro that
# directory is served by systemd-xdg-autostart-generator, and in a full
# desktop it is served by that desktop's own session manager. Where
# neither is true, dropping the file in would look like an install and do
# nothing — so this prints the line to add instead.
autostart_reader() {
    if command -v systemctl >/dev/null 2>&1 &&
       systemctl --user is-active xdg-desktop-autostart.target >/dev/null 2>&1; then
        echo "systemd (xdg-desktop-autostart.target)"
        return 0
    fi
    case ":${XDG_CURRENT_DESKTOP:-}:" in
        *:GNOME:*|*:KDE:*|*:XFCE:*|*:X-Cinnamon:*|*:MATE:*|*:LXQt:*|*:Budgie:*|*:Pantheon:*|*:Deepin:*)
            echo "${XDG_CURRENT_DESKTOP} session manager"
            return 0 ;;
    esac
    return 1
}

compositor_hint() {
    if [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
        say "  hyprland.conf:  exec-once = $BIN_DIR/$APP"
    elif [ -n "${SWAYSOCK:-}" ]; then
        say "  sway config:    exec $BIN_DIR/$APP"
    else
        say "  your compositor's startup config:  $BIN_DIR/$APP"
    fi
}

if [ "$want_autostart" = 0 ]; then
    say "skipped autostart (--no-autostart)"
elif reader=$(autostart_reader); then
    mkdir -p "$AUTOSTART_DIR"
    write_entry "$AUTOSTART_DIR/$DESKTOP_ID.desktop"
    say "installed $AUTOSTART_DIR/$DESKTOP_ID.desktop — read by $reader"
    # The generator only sees files that existed when it last ran.
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload 2>/dev/null || true
    fi
else
    say ""
    say "No XDG autostart in this session, so nothing was installed for it."
    say "Add this line instead:"
    compositor_hint
fi

# --- afterwards -----------------------------------------------------------

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) say ""; say "note: $BIN_DIR is not on PATH; start it with $BIN_DIR/$APP" ;;
esac

say ""
say "Start it now:  $BIN_DIR/$APP"
say "The system proxy stays off until you turn it on inside $APP."
say "Data directory: $DATA_DIR"
