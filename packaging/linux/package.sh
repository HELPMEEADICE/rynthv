#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

arch=$1
binary_dir=$2
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
case "$arch" in
    x86_64) machine='Advanced Micro Devices X86-64' ;;
    aarch64) machine='AArch64' ;;
    *) echo "Unsupported AppImage architecture: $arch" >&2; exit 1 ;;
esac

require_machine() {
    local actual
    actual=$(readelf -h -- "$1" | awk -F: '/Machine:/ { sub(/^[[:space:]]+/, "", $2); print $2 }')
    if [[ "$actual" != "$machine" ]]; then
        echo "Expected $machine in $1, found $actual" >&2
        return 1
    fi
}

stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
appdir="$stage/AppDir"
desktop="$appdir/usr/share/applications/rynthv.desktop"
icon="$appdir/usr/share/icons/hicolor/256x256/apps/rynthv.png"
mkdir -p "$root/dist" "$appdir/usr/bin" "$(dirname -- "$desktop")" "$(dirname -- "$icon")"
for name in rynthv rynthv-synth; do
    require_machine "$binary_dir/$name"
    install -m 755 "$binary_dir/$name" "$appdir/usr/bin/$name"
done
install -m 644 "$root/assets/branding/app-icon.png" "$icon"
cat > "$desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=RynthV
Comment=Vocal synthesis editor
Exec=rynthv
Icon=rynthv
Categories=AudioVideo;Audio;Music;
Terminal=false
DESKTOP

linuxdeploy="$stage/linuxdeploy.AppImage"
curl --fail --location --retry 3 \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$arch.AppImage" \
    -o "$linuxdeploy"
chmod +x "$linuxdeploy"
image="$root/dist/rynthv-linux-$arch.AppImage"
ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 LDAI_OUTPUT="$image" "$linuxdeploy" \
    --appdir "$appdir" \
    --executable "$appdir/usr/bin/rynthv" \
    --executable "$appdir/usr/bin/rynthv-synth" \
    --desktop-file "$desktop" \
    --icon-file "$icon" \
    --output appimage

require_machine "$image"
# Exercise the AppImage runtime's extraction path without launching RynthV.
(
    cd -- "$stage"
    "$image" --appimage-extract >/dev/null
    test -x squashfs-root/AppRun
    require_machine squashfs-root/usr/bin/rynthv
    require_machine squashfs-root/usr/bin/rynthv-synth
)
