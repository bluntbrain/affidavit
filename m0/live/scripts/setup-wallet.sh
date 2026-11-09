#!/usr/bin/env bash
# Fresh wallet pointed at the local sequencer, with the funded test account imported and a
# private "verifier" account created and funded.
# Usage: setup-wallet.sh <amount to move to the private verifier account>
set -euo pipefail
source "$(dirname "$0")/env.sh"
AMOUNT="${1:-1000000000}"; export RISC0_DEV_MODE="${2:-1}"; [ "$RISC0_DEV_MODE" = "1" ] || unset RISC0_DEV_MODE
rm -rf "$LEE_WALLET_HOME_DIR"; mkdir -p "$LEE_WALLET_HOME_DIR"
W="$WALLET_BIN"
"$W" change-network local
"$W" check-health
"$W" account import public --private-key "$TEST_PRIVATE_KEY"
"$W" account new private --label verifier
"$W" account list
PAYER=$("$W" account list 2>/dev/null | grep -oE "Public/[1-9A-HJ-NP-Za-km-z]{32,44}" | head -1)
VERIFIER=$("$W" account list 2>/dev/null | grep -oE "Private/[1-9A-HJ-NP-Za-km-z]{32,44}" | head -1)
echo "payer=$PAYER verifier=$VERIFIER"
"$W" auth-transfer send --from "$PAYER" --to "$VERIFIER" --amount "$AMOUNT"
"$W" account sync-private || true
"$W" account get --scope balance --account-id "$VERIFIER" || true
echo "$PAYER" > "$LIVE/payer.txt"; echo "$VERIFIER" > "$LIVE/verifier.txt"
