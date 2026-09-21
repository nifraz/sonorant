#!/usr/bin/env bash
# Builds a .deb for the machine it runs on, or for a target given with --target.
#
#   packaging/deb/build.sh                                   this machine's architecture
#   packaging/deb/build.sh --target aarch64-unknown-linux-gnu --no-build
#   packaging/deb/build.sh --version 0.2.0 --out /tmp/debs
#
# There is no cross-compilation here on purpose: the release workflow builds the arm64
# package on an arm64 runner, which keeps one toolchain and one set of PipeWire headers
# per architecture rather than a sysroot nobody tests.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
target=""
version=""
out="$root/target/deb"
build=1

while [ $# -gt 0 ]; do
    case "$1" in
        --target) target="$2"; shift 2 ;;
        --version) version="$2"; shift 2 ;;
        --out) out="$2"; shift 2 ;;
        --no-build) build=0; shift ;;   # Use a binary that is already built.
        -h|--help) sed -n '2,9p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

# The version the crate carries, unless one was asked for. The release workflow stamps
# Cargo.toml from the tag before it gets here, so both routes agree.
if [ -z "$version" ]; then
    version=$(sed -n '/^\[workspace\.package\]/,/^\[/p' "$root/Cargo.toml" |
        sed -n 's/^version = "\(.*\)"/\1/p' | head -1)
fi
[ -n "$version" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }

machine=${target%%-*}
[ -n "$target" ] || machine=$(uname -m)
case "$machine" in
    x86_64) arch=amd64 ;;
    aarch64) arch=arm64 ;;
    *) echo "no Debian architecture known for $machine" >&2; exit 1 ;;
esac

if [ "$build" = 1 ]; then
    if [ -n "$target" ]; then
        cargo build --release --locked -p sonorant --target "$target"
    else
        cargo build --release --locked -p sonorant
    fi
fi
binary="$root/target/${target:+$target/}release/sonorant"
[ -x "$binary" ] || { echo "no binary at $binary" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
# mktemp makes it private; the package's own root has to be readable by everyone.
chmod 755 "$stage"
app=io.github.nifraz.Sonorant

install -Dm755 "$binary" "$stage/usr/bin/sonorant"
install -Dm644 "$root/packaging/linux/$app.desktop" "$stage/usr/share/applications/$app.desktop"
install -Dm644 "$root/packaging/linux/$app.metainfo.xml" \
    "$stage/usr/share/metainfo/$app.metainfo.xml"
for size in 16 32 48 64 128 256; do
    install -Dm644 "$root/packaging/icons/sonorant-$size.png" \
        "$stage/usr/share/icons/hicolor/${size}x${size}/apps/$app.png"
done
install -Dm644 "$root/packaging/icons/sonorant.svg" \
    "$stage/usr/share/icons/hicolor/scalable/apps/$app.svg"
install -Dm644 "$root/packaging/deb/copyright" "$stage/usr/share/doc/sonorant/copyright"
install -Dm644 "$root/README.md" "$stage/usr/share/doc/sonorant/README.md"

# Debian's changelog, which is the packaging's own and not the project's history.
mkdir -p "$stage/usr/share/doc/sonorant"
{
    echo "sonorant ($version) unstable; urgency=medium"
    echo
    echo "  * Built from the sonorant repository at version $version."
    echo
    echo " -- Nifraz <nifraz@live.com>  $(date -R)"
} | gzip -9n > "$stage/usr/share/doc/sonorant/changelog.gz"
chmod 644 "$stage/usr/share/doc/sonorant/changelog.gz"

# Kilobytes of installed files, which is what the field means.
size=$(du -ks "$stage" | cut -f1)

mkdir -p "$stage/DEBIAN"
cat > "$stage/DEBIAN/control" <<EOF
Package: sonorant
Version: $version
Architecture: $arch
Maintainer: Nifraz <nifraz@live.com>
Installed-Size: $size
Section: sound
Priority: optional
Homepage: https://github.com/nifraz/sonorant
Depends: libc6, libgcc-s1, libpipewire-0.3-0
Recommends: libwayland-client0, libxkbcommon0, libvulkan1, mesa-vulkan-drivers
Description: Real-time spectrogram, spectrum and loudness visualiser
 Sonorant draws what the computer is playing: a scrolling spectrogram of both
 channels, the spectrum beside it, and broadcast loudness, true peak, dynamic
 range, tempo and brightness underneath. It captures the whole system or one
 app through PipeWire, and follows whatever player is running over MPRIS.
 .
 It is the standalone rewrite of the Nostalgia+ plugin for MusicBee, drawn on
 the GPU with its own shaders.
EOF

mkdir -p "$out"
package="$out/sonorant_${version}_${arch}.deb"
# --root-owner-group so the files come out owned by root without needing fakeroot.
dpkg-deb --root-owner-group --build "$stage" "$package" >/dev/null
echo "$package"
