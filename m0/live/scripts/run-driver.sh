#!/usr/bin/env bash
# Run the live driver. Usage: run-driver.sh <out-dir-name> <RISC0_DEV_MODE 1|0> [LIVE_UPDATES]
set -euo pipefail
source "$(dirname "$0")/env.sh"
OUT="$LIVE/$1"; DEV="${2:-1}"; mkdir -p "$OUT"
if [ "$DEV" = "1" ]; then export RISC0_DEV_MODE=1; else unset RISC0_DEV_MODE; fi
export LIVE_UPDATES="${3:-7}" AFFIDAVIT_LIVE_OUT="$OUT" RUST_LOG=info,risc0_zkvm=warn
PAYER=$(sed 's#^Public/##' "$LIVE/payer.txt"); VERIFIER=$(sed 's#^Private/##' "$LIVE/verifier.txt")
"$DRIVER_BIN" "$PROGRAM_ELF" "$PAYER" "$VERIFIER" 2> "$OUT/driver-stderr.log" | tee "$OUT/driver-stdout.log"
