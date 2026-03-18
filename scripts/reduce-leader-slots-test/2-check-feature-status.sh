#!/usr/bin/env bash
#
# Step 2: Check the reduce_consecutive_leader_slots feature status.
# Run this while the multinode demo validators are running.
#
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

PROFILE="${CARGO_BUILD_PROFILE:-release}"
SOLANA_BIN="$REPO_ROOT/target/$PROFILE/solana"
RPC_URL="${RPC_URL:-http://localhost:8899}"

# Feature ID for reduce_consecutive_leader_slots (must match feature-keypair.json pubkey)
FEATURE_ID="9N4TN7bBtviskXWRo8pFcvp4caa6pPUQTx9BRWyeYLo"

if [[ ! -x "$SOLANA_BIN" ]]; then
  echo "Error: $SOLANA_BIN not found. Build the project first (e.g. cargo build --release)."
  exit 1
fi

echo "Checking feature status..."
echo "  RPC: $RPC_URL"
echo "  Feature: reduce_consecutive_leader_slots ($FEATURE_ID)"
echo ""

# Check our specific feature
echo "--- Our feature ---"
"$SOLANA_BIN" feature status "$FEATURE_ID" --url "$RPC_URL" || true

echo ""
echo "--- All features (first 20) ---"
"$SOLANA_BIN" feature status --url "$RPC_URL" 2>/dev/null | head -30 || true
