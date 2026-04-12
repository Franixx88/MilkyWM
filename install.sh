#!/bin/sh
# Install MilkyWM system-wide.
#
# Usage:
#   ./install.sh          # build release + install
#   ./install.sh --skip-build   # install without rebuilding
#   ./install.sh --uninstall    # remove installed files

set -e

PREFIX="${PREFIX:-/usr/local}"
SESSION_DIR="/usr/share/wayland-sessions"

case "${1:-}" in
    --uninstall)
        echo "Removing MilkyWM..."
        sudo rm -f "$PREFIX/bin/milkywm"
        sudo rm -f "$PREFIX/bin/milkyctl"
        sudo rm -f "$PREFIX/bin/milkywm-session"
        sudo rm -f "$SESSION_DIR/milkywm.desktop"
        echo "Done."
        exit 0
        ;;
    --skip-build)
        echo "Skipping build..."
        ;;
    *)
        echo "Building MilkyWM (release)..."
        cargo build --release
        echo "Build complete."
        ;;
esac

echo "Installing to $PREFIX/bin/ ..."

sudo install -Dm755 target/release/milkywm     "$PREFIX/bin/milkywm"
sudo install -Dm755 target/release/milkyctl     "$PREFIX/bin/milkyctl"
sudo install -Dm755 session/milkywm-session     "$PREFIX/bin/milkywm-session"
sudo install -Dm644 session/milkywm.desktop     "$SESSION_DIR/milkywm.desktop"

echo ""
echo "Installed:"
echo "  $PREFIX/bin/milkywm"
echo "  $PREFIX/bin/milkyctl"
echo "  $PREFIX/bin/milkywm-session"
echo "  $SESSION_DIR/milkywm.desktop"
echo ""
echo "MilkyWM should now appear in your display manager (GDM / SDDM)."
echo "Select it from the session picker and log in."
