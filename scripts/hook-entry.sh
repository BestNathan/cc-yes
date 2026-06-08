#!/bin/bash
set -euo pipefail

if [ "${CC_YES_ENABLED:-1}" = "0" ]; then
  exit 0
fi

BIN="${CLAUDE_PLUGIN_ROOT}/bin/cc-yes"
if [ ! -x "$BIN" ]; then
  exit 0
fi

exec "$BIN" hook "${1:-pretooluse}"
