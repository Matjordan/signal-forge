#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked --bin signal-forge
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
name="signal-forge-${version}-linux-x86_64"
[[ $(uname -m) == x86_64 ]] || { echo 'This package target requires x86_64 Linux' >&2; exit 1; }
mkdir -p dist
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/$name/bin" "$staging/$name/share/applications" "$staging/$name/share/doc/signal-forge"
install -m755 target/release/signal-forge "$staging/$name/bin/"
install -m644 packaging/signal-forge.desktop "$staging/$name/share/applications/"
cp README.md "$staging/$name/share/doc/signal-forge/"
cp -R docs "$staging/$name/share/doc/signal-forge/"
if [[ -f a_detailed_widescreen_dark_themed_desktop_applicat.png ]]; then
    cp a_detailed_widescreen_dark_themed_desktop_applicat.png "$staging/$name/share/doc/signal-forge/"
fi
install -m755 packaging/install.sh "$staging/$name/"
ldd target/release/signal-forge > "$staging/$name/share/doc/signal-forge/linked-libraries.txt"
tar -C "$staging" -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
