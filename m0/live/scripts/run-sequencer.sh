#!/usr/bin/env bash
# Start the standalone LEZ v0.4.0-rc1 sequencer (mock Bedrock) in the background.
# Usage: run-sequencer.sh <home-dir-name> <RISC0_DEV_MODE value: 1 or 0>
set -euo pipefail
source "$(dirname "$0")/env.sh"
HOME_DIR="$LIVE/$1"; DEV="${2:-1}"
cd "$LEZ_REPO"
if [ ! -f "$HOME_DIR/sequencer_config.json" ]; then
  "$SEQ_BIN" setup "$HOME_DIR" --config lez/sequencer/service/networks/local.json \
    --funding-public-key "$BEDROCK_FUNDING_PUBLIC_KEY" --create-channel
fi
if [ "$DEV" = "1" ]; then export RISC0_DEV_MODE=1; else unset RISC0_DEV_MODE; fi
export RUST_LOG=info,risc0_zkvm=warn,kameo=warn
nohup "$SEQ_BIN" start "$HOME_DIR" > "$HOME_DIR.log" 2>&1 &
echo "sequencer pid $! (home $HOME_DIR, RISC0_DEV_MODE=${RISC0_DEV_MODE:-unset}), log $HOME_DIR.log"
