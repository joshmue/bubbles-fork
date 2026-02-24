#!/bin/bash
#
# prebuild.bash — Populate prebuilt/ for local (non-CI) Flatpak builds
#
# Installs socat and qemu-img inside a Debian Trixie container via apt
# (which verifies package signatures), then copies the binaries and their
# runtime library dependencies out. Same approach as the build-tools
# job in .github/workflows/app.yml.
#
# Usage:
#   CROSVM=~/bubbles/crosvm ./prebuild.bash
#
# Environment variables:
#   CROSVM  - path to a pre-built crosvm binary (required)
#
# Requirements: podman, curl

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PREBUILT_DIR="$SCRIPT_DIR/prebuilt"
CONTAINER_NAME="bubbles-prebuild-$$"

cleanup() {
    podman rm -f "$CONTAINER_NAME" 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "$PREBUILT_DIR" "$PREBUILT_DIR/lib"

# ---------------------------------------------------------------------------
# crosvm — must be provided; build instructions in .github/workflows/app.yml
# ---------------------------------------------------------------------------
if [ -n "${CROSVM:-}" ]; then
    echo "==> crosvm: copying from ${CROSVM}"
    install -m755 "$CROSVM" "$PREBUILT_DIR/crosvm"
    echo "    → prebuilt/crosvm"
else
    echo "ERROR: crosvm binary not found." >&2
    echo "  Set CROSVM=/path/to/binary, e.g.:" >&2
    echo "    CROSVM=~/bubbles/crosvm $0" >&2
    echo "  Build instructions: .github/workflows/app.yml (build-crosvm job)" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# socat, qemu-img, and runtime libraries — install in Debian Trixie container
# via apt (verifies package GPG signatures), then copy binaries and their
# non-system shared library dependencies out.
# ---------------------------------------------------------------------------
echo "==> Setting up Debian Trixie container..."

podman run -d --name "$CONTAINER_NAME" debian:trixie sleep infinity
podman exec "$CONTAINER_NAME" sh -c \
    'apt-get update && apt-get install -y --no-install-recommends socat qemu-utils'

# Copy binaries
echo "==> Copying binaries..."
podman cp "$CONTAINER_NAME:/usr/bin/socat1"   "$PREBUILT_DIR/socat"
podman cp "$CONTAINER_NAME:/usr/bin/qemu-img" "$PREBUILT_DIR/qemu-img"
chmod +x "$PREBUILT_DIR/socat" "$PREBUILT_DIR/qemu-img"
echo "    → prebuilt/socat"
echo "    → prebuilt/qemu-img"

# Copy runtime library dependencies (excluding glibc/base system libs)
echo "==> Copying runtime libraries..."

# Libraries that are part of glibc or universally present — skip these
SYSTEM_LIBS="linux-vdso|ld-linux|libc\.so|libm\.so|libdl\.so|librt\.so|libpthread\.so|libgcc_s\.so|libstdc\+\+"

DEPS=$(podman exec "$CONTAINER_NAME" sh -c \
    'ldd /usr/bin/socat1 /usr/bin/qemu-img 2>/dev/null \
     | grep "=> /" | awk "{print \$3}" | sort -u')

for lib in $DEPS; do
    libname=$(basename "$lib")
    if echo "$libname" | grep -qE "$SYSTEM_LIBS"; then
        continue
    fi
    podman cp "$CONTAINER_NAME:$lib" "$PREBUILT_DIR/lib/$libname"
    echo "    → prebuilt/lib/$libname"
done

# ---------------------------------------------------------------------------
# cargo-sources.json — Flatpak needs this for offline Cargo builds
# Run generator inside the container using apt-provided Python packages
# (avoids needing pip/aiohttp on the host).
# ---------------------------------------------------------------------------
if [ -f "$SCRIPT_DIR/cargo-sources.json" ]; then
    echo "==> cargo-sources.json already exists, skipping"
else
    echo "==> Generating cargo-sources.json (inside container)..."
    podman exec "$CONTAINER_NAME" sh -c \
        'apt-get install -y --no-install-recommends python3 python3-aiohttp python3-tomlkit curl 2>/dev/null'
    curl -fsSL -o "$SCRIPT_DIR/.flatpak-cargo-generator.py" \
        https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/4d5e760321236bd96fc1c6db9ec94c336600c114/cargo/flatpak-cargo-generator.py
    podman cp "$SCRIPT_DIR/Cargo.lock"                    "$CONTAINER_NAME:/tmp/Cargo.lock"
    podman cp "$SCRIPT_DIR/.flatpak-cargo-generator.py"   "$CONTAINER_NAME:/tmp/flatpak-cargo-generator.py"
    rm -f "$SCRIPT_DIR/.flatpak-cargo-generator.py"
    podman exec "$CONTAINER_NAME" \
        python3 /tmp/flatpak-cargo-generator.py /tmp/Cargo.lock -o /tmp/cargo-sources.json
    podman cp "$CONTAINER_NAME:/tmp/cargo-sources.json" "$SCRIPT_DIR/cargo-sources.json"
    echo "    → cargo-sources.json"
fi

# ---------------------------------------------------------------------------
echo ""
echo "prebuilt/ ready:"
ls -lhR "$PREBUILT_DIR/"
echo ""
echo "To build the Flatpak:"
echo "  cd $(basename "$SCRIPT_DIR")"
echo "  flatpak-builder --user --install --force-clean build-dir de.gonicus.Bubbles.json"
