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
mkdir -p "$staging/$name/share/signal-forge"
install -m755 packaging/uninstall.sh "$staging/$name/share/signal-forge/"
for size in 16 24 32 48 64 128 256 512; do
    destination="$staging/$name/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$destination"
    install -m644 "assets/icons/signal-forge-${size}.png" "$destination/signal-forge.png"
done
ldd target/release/signal-forge > "$staging/$name/share/doc/signal-forge/linked-libraries.txt"
python3 - "$staging/$name" <<'PYMANIFEST'
import json
from pathlib import Path
import sys
root = Path(sys.argv[1])
manifest = root / 'share/signal-forge/install-manifest.json'
files = sorted(str(path.relative_to(root)) for path in (root / 'share').rglob('*') if path.is_file())
files.append(str(manifest.relative_to(root)))
manifest.write_text(json.dumps({'owner': 'signal-forge', 'version': 1, 'files': files}, indent=2) + '\n')
PYMANIFEST
tar -C "$staging" -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
