#!/usr/bin/env bash
set -euo pipefail
prefix=$(cd "$(dirname "$0")/../.." && pwd)
exec "$prefix/bin/signal-forge" --uninstall-prefix "$prefix"
