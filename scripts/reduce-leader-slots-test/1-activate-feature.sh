#!/usr/bin/env bash
#
# Step 1: Activate the reduce_consecutive_leader_slots feature.
# Run this while the multinode demo validators are running.
#
# The feature starts inactive. This script submits the activation transaction
# using the feature's keypair (required for feature activation).
#
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

PROFILE="${CARGO_BUILD_PROFILE:-release}"
SOLANA_BIN="$REPO_ROOT/target/$PROFILE/solana"
KEYGEN_BIN="$REPO_ROOT/target/$PROFILE/solana-keygen"
RPC_URL="${RPC_URL:-http://localhost:8899}"
KEYPAIR_FILE="$SCRIPT_DIR/feature-keypair.json"

if [[ ! -x "$SOLANA_BIN" ]]; then
  echo "Error: $SOLANA_BIN not found. Build the project first (e.g. cargo build --release)."
  exit 1
fi

if [[ ! -f "$KEYPAIR_FILE" ]]; then
  echo "Error: Feature keypair not found at $KEYPAIR_FILE"
  echo "Run: solana-keygen new -o $KEYPAIR_FILE --no-passphrase"
  echo "Then update the feature ID in feature-set to match the pubkey."
  exit 1
fi

# Fee payer for the activation tx
PAYER_KEYPAIR="$SCRIPT_DIR/payer-keypair.json"
if [[ ! -f "$PAYER_KEYPAIR" ]]; then
  echo "Creating fee payer keypair..."
  "$KEYGEN_BIN" new -o "$PAYER_KEYPAIR" --no-passphrase --force
fi
echo "Funding fee payer..."
"$SOLANA_BIN" airdrop 1 --url "$RPC_URL" --keypair "$PAYER_KEYPAIR" || true

echo "Activating reduce_consecutive_leader_slots feature..."
echo "  RPC: $RPC_URL"
echo "  Keypair: $KEYPAIR_FILE"
echo ""

# --yolo --yolo forces activation (bypasses stake/RPC sanity checks for local testing)
"$SOLANA_BIN" feature activate "$KEYPAIR_FILE" development --url "$RPC_URL" \
  --fee-payer "$PAYER_KEYPAIR" --yolo --yolo

echo ""
echo "Activation submitted. Run 2-check-feature-status.sh to verify."
