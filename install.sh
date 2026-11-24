#!/bin/sh
# Installs a test build of Create for Horizon into an instance of its own, then builds it (macOS and Linux;
# install.ps1 is the Windows one).
#
#   sh install.sh
#   sh install.sh --instance my-create --release v0.1.0-test3
#   sh install.sh --file /path/to/create.hmod                 (a local copy instead; needs python3)
#   sh install.sh --launcher "/Applications/Horizon Launcher.app/Contents/MacOS/horizon-launcher"
#
# Needs the Horizon Launcher (https://playhorizon.dev), opened once and signed in with an account that owns
# Minecraft: Java Edition. The mod is Rust source: the launcher compiles it on this computer. The first build
# downloads Rust and takes about 20 to 30 minutes and a few GB; later ones are quicker.
set -eu

INSTANCE="create-test"
RELEASE="v0.1.0-test3"
FILE=""
LAUNCHER="${HORIZON_LAUNCHER:-}"
REPO="https://github.com/leanwrldd/Create-Horizon"
# The Horizon this build is made for. The mod pins the exact game build (horizon-mod.toml [native]); this nightly
# provides the game files the build reuses.
HORIZON="nightly-2026-11-22-f1bb66f"

while [ $# -gt 0 ]; do
    case "$1" in
        --instance) INSTANCE="$2"; shift 2 ;;
        --release) RELEASE="$2"; shift 2 ;;
        --file) FILE="$2"; shift 2 ;;
        --launcher) LAUNCHER="$2"; shift 2 ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "Unknown option: $1 (see the top of this script)"; exit 1 ;;
    esac
done

# Finds the launcher's command-line program: the --launcher path or $HORIZON_LAUNCHER, `horizon-launcher` on the
# PATH, then the app bundles macOS keeps in Applications, then the usual Linux places.
find_launcher() {
    if [ -n "$LAUNCHER" ]; then
        [ -x "$LAUNCHER" ] && { echo "$LAUNCHER"; return 0; }
        return 1
    fi
    if command -v horizon-launcher >/dev/null 2>&1; then
        command -v horizon-launcher
        return 0
    fi
    for app in "/Applications"/*[Hh]orizon*.app "$HOME/Applications"/*[Hh]orizon*.app; do
        [ -d "$app/Contents/MacOS" ] || continue
        for bin in "$app/Contents/MacOS"/horizon-launcher* "$app/Contents/MacOS"/*; do
            if [ -f "$bin" ] && [ -x "$bin" ]; then
                echo "$bin"
                return 0
            fi
        done
    done
    for bin in "$HOME/.local/bin/horizon-launcher" "$HOME/Applications"/horizon-launcher*.AppImage \
               "$HOME/Downloads"/horizon-launcher*.AppImage /opt/horizon-launcher/horizon-launcher; do
        if [ -f "$bin" ] && [ -x "$bin" ]; then
            echo "$bin"
            return 0
        fi
    done
    return 1
}

if ! LAUNCHER="$(find_launcher)"; then
    echo "The Horizon Launcher wasn't found. Get it from https://playhorizon.dev, open it once and sign in, then run"
    echo "this script again. If it's installed somewhere unusual, pass its program with --launcher PATH (on macOS"
    echo "that's inside the app: \"/Applications/<the app>.app/Contents/MacOS/<the program>\")."
    exit 1
fi

run() {
    echo ">> horizon-launcher $*"
    "$LAUNCHER" "$@"
}

# The instance's folder, from the launcher's JSON list (python3), or nothing.
instance_home() {
    command -v python3 >/dev/null 2>&1 || return 0
    "$LAUNCHER" instance list --json 2>/dev/null | python3 -c '
import json, sys
name = sys.argv[1]
try:
    items = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for item in items if isinstance(items, list) else []:
    if item.get("id") == name or item.get("name") == name:
        print(item.get("home", ""))
        break
' "$1"
}

instance_exists() {
    "$LAUNCHER" instance list --json 2>/dev/null | grep -q "\"$1\""
}

echo ""
echo "Create for Horizon: test build"
echo ""
echo "  - This is a test build. Expect bugs, and keep it away from worlds you care about: it goes into its own"
echo "    instance, '$INSTANCE'. A world opened later without the mod loses its Create blocks."
echo "  - It's a native mod: Rust code that runs with your rights, outside any sandbox, and isn't reviewed by"
echo "    the Horizon registry. The source is at $REPO."
echo "  - The first build downloads Rust and takes about 20 to 30 minutes."
echo "  - Launcher: $LAUNCHER"
echo ""
printf "Install it? Type yes to go on: "
read -r answer
if [ "$answer" != "yes" ]; then
    echo "Nothing was installed."
    exit 0
fi

# 1. The Horizon version, with resources prepared from your own Minecraft.
run install "$HORIZON" || true
run prepare --version "$HORIZON"

# 2. An instance of its own, kept on that version.
if ! instance_exists "$INSTANCE"; then
    run instance create "$INSTANCE"
fi
run instance version "$INSTANCE" "$HORIZON"

# 3. The mod: from the GitHub release (its checksum checked by the launcher), or a local file.
if [ -n "$FILE" ]; then
    [ -f "$FILE" ] || { echo "No file at $FILE"; exit 1; }
    HOME_DIR="$(instance_home "$INSTANCE")"
    if [ -z "$HOME_DIR" ]; then
        echo "Couldn't find the instance's folder (installing a local file needs python3 to read the launcher's list)."
        exit 1
    fi
    mkdir -p "$HOME_DIR/horizon-native-mods"
    cp "$FILE" "$HOME_DIR/horizon-native-mods/create.hmod"
    LIST="$HOME_DIR/horizon-mods.txt"
    if [ -f "$LIST" ]; then
        grep -v '^mod\.create:' "$LIST" > "$LIST.tmp" || true
        mv "$LIST.tmp" "$LIST"
    fi
    echo "mod.create:true" >> "$LIST"
    echo "Copied $FILE into $HOME_DIR/horizon-native-mods"
else
    run mods add-github "$REPO/releases/tag/$RELEASE" --instance "$INSTANCE" --trust-native-code
fi

# 4. Compile it for the instance.
echo ""
echo "Building. The first time takes about 20 to 30 minutes; leave this terminal open."
started=$(date +%s)
run native-build --instance "$INSTANCE" --version "$HORIZON" --trust-native-code
took=$(( $(date +%s) - started ))

echo ""
echo "Done in $((took / 60)) min $((took % 60)) s. Play it from the launcher (instance '$INSTANCE'), or with:"
echo "  \"$LAUNCHER\" play --instance $INSTANCE --world create-test --creative"
echo ""
echo "Found a bug? Report it at $REPO/issues/new/choose"
HOME_DIR="$(instance_home "$INSTANCE")"
if [ -n "$HOME_DIR" ]; then
    echo "Your log is in $HOME_DIR/logs/latest.log"
else
    echo "Your log is the instance's logs/latest.log (the launcher's Open folder shows it)."
fi
