#!/usr/bin/env bash
# Local packaging script for presence-switch.
#
# Mirrors the steps run by .github/workflows/package.yml so you can reproduce
# CI builds (or just inspect failures) without pushing.
#
# Usage:
#   scripts/package.sh rpm      Build the .rpm into target/generate-rpm/
#   scripts/package.sh msi      Cross-compile the Windows binary and build the .msi
#   scripts/package.sh all      Both
#
# Required for RPM:  cargo, cargo-generate-rpm (cargo install cargo-generate-rpm)
# Required for MSI:  mingw64-gcc msitools (provides wixl)
#                    plus rustup target x86_64-pc-windows-gnu

set -euo pipefail

# Ensure tooling installed by rustup is findable, regardless of whether the
# user has ~/.cargo/bin in their shell rc.
export PATH="$HOME/.cargo/bin:$PATH"

cd "$(dirname "$0")/.."
ROOT=$(pwd)
NAME=presence-switch
VERSION=$(grep -E '^version *= *' Cargo.toml | head -n1 | cut -d'"' -f2)

if SHORT_SHA=$(git rev-parse --short=7 HEAD 2>/dev/null); then
    if ! git diff-index --quiet HEAD -- 2>/dev/null; then
        SHORT_SHA="${SHORT_SHA}.dirty"
    fi
else
    SHORT_SHA="nogit"
fi

LOCAL_RELEASE="0.local.${SHORT_SHA}"

log()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "$1 not found. $2"; }

build_rpm() {
    log "Building RPM   name=${NAME} version=${VERSION} release=${LOCAL_RELEASE}"

    need cargo              "Install rustup: https://rustup.rs"
    need cargo-generate-rpm "Install: cargo install cargo-generate-rpm --locked"

    log "Compiling release binary"
    cargo build --release --locked

    log "Generating RPM"
    mkdir -p target/generate-rpm
    cargo generate-rpm \
        --output "target/generate-rpm/${NAME}-${VERSION}-${LOCAL_RELEASE}.x86_64.rpm" \
        -s "release = \"${LOCAL_RELEASE}\""

    local built="${ROOT}/target/generate-rpm/${NAME}-${VERSION}-${LOCAL_RELEASE}.x86_64.rpm"
    log "Built RPM: ${built}"
}

build_msi() {
    log "Building MSI   name=${NAME} version=${VERSION} sha=${SHORT_SHA}"

    need cargo                  "Install rustup: https://rustup.rs"
    need x86_64-w64-mingw32-gcc "Install: sudo dnf install mingw64-gcc"
    need wixl                   "Install: sudo dnf install msitools"

    if ! rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-gnu$'; then
        log "Adding rustup target x86_64-pc-windows-gnu"
        rustup target add x86_64-pc-windows-gnu
    fi

    log "Cross-compiling presence-switch.exe"
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
        cargo build --release --locked --target x86_64-pc-windows-gnu

    log "Linking MSI"
    mkdir -p target/wix
    local out="target/wix/${NAME}-${VERSION}-${SHORT_SHA}.msi"
    # -D Version threads the Cargo.toml version into the MSI's ProductVersion
    # so MajorUpgrade detection stays correct as the project version bumps.
    wixl \
        --arch x64 \
        -D "Version=${VERSION}" \
        -D "ExePath=target/x86_64-pc-windows-gnu/release/${NAME}.exe" \
        --output "${out}" \
        wix/main.wxs
    log "Built MSI: ${ROOT}/${out}"
}

usage() {
    cat <<EOF
Local packaging script for presence-switch.

Mirrors the steps run by .github/workflows/package.yml so you can reproduce
CI builds (or just inspect failures) without pushing.

Usage:
  scripts/package.sh rpm      Build the .rpm into target/generate-rpm/
  scripts/package.sh msi      Cross-compile the Windows binary and build the .msi
  scripts/package.sh all      Both

Required for RPM:  cargo, cargo-generate-rpm (cargo install cargo-generate-rpm)
Required for MSI:  mingw64-gcc msitools (provides wixl)
                   plus rustup target x86_64-pc-windows-gnu
EOF
}

case "${1:-}" in
    rpm) build_rpm ;;
    msi) build_msi ;;
    all) build_rpm; build_msi ;;
    -h|--help) usage; exit 0 ;;
    "") usage; exit 1 ;;
    *) fail "Unknown command: $1 (try 'rpm', 'msi', 'all', or --help)" ;;
esac
