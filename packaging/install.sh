#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
prefix="${1:-${HOME}/.local}"
# The packaged executable shares its owned-file installer with self-update.
bin/signal-forge --install-package "$PWD" "$prefix"
printf 'Installed Signal Forge in %s/bin\n' "$prefix"
