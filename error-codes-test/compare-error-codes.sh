#!/usr/bin/env bash

set -euo pipefail

SIGNATURE="${1:-}"
NETWORK="${NETWORK:-mainnet}"
LOCAL_RPC="${LOCAL_RPC:-http://localhost:8899}"

case "$NETWORK" in
    mainnet|mainnet-beta)
        PUBLIC_RPC="${PUBLIC_RPC:-https://api.mainnet-beta.solana.com}"
        ;;
    testnet)
        PUBLIC_RPC="${PUBLIC_RPC:-https://api.testnet.solana.com}"
        ;;
    *)
        echo "Unknown NETWORK=$NETWORK (use mainnet or testnet)"
        exit 1
        ;;
esac

if [[ -z "$SIGNATURE" ]]; then
    echo "Usage: $0 <signature>"
    echo ""
    echo "Environment variables:"
    echo "  NETWORK     mainnet (default) or testnet"
    echo "  LOCAL_RPC   local validator RPC (default: http://localhost:8899)"
    echo "  PUBLIC_RPC  public RPC endpoint (default: auto from NETWORK)"
    exit 1
fi

echo "=== Error Codes Consensus Test ==="
echo "Network:    $NETWORK"
echo "Local RPC:  $LOCAL_RPC"
echo "Public RPC: $PUBLIC_RPC"
echo ""
echo "Signature: $SIGNATURE"
echo ""
echo "Waiting for finalization (~15s)..."
sleep 15

# Query both RPC endpoints for the same transaction
echo ""
echo "=== Local RPC (patched validator) ==="
LOCAL_ERR=$(curl -s "$LOCAL_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\", \"id\": 1,
        \"method\": \"getTransaction\",
        \"params\": [
            \"$SIGNATURE\",
            {\"encoding\": \"json\",
             \"commitment\": \"finalized\",
             \"maxSupportedTransactionVersion\": 0}
        ]
    }" | jq '.result.meta.err')
echo "$LOCAL_ERR"

echo ""
echo "=== Public RPC (stock validators) ==="
PUBLIC_ERR=$(curl -s "$PUBLIC_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\", \"id\": 1,
        \"method\": \"getTransaction\",
        \"params\": [
            \"$SIGNATURE\",
            {\"encoding\": \"json\",
             \"commitment\": \"finalized\",
             \"maxSupportedTransactionVersion\": 0}
        ]
    }" | jq '.result.meta.err')
echo "$PUBLIC_ERR"

# Compare
echo ""
echo "=== Comparison ==="
if [[ "$LOCAL_ERR" == "null" && "$PUBLIC_ERR" == "null" ]]; then
    echo "Both returned null. The transaction may not"
    echo "have finalized yet, or was not included in a"
    echo "block. Re-run with:"
    echo "  $0 $SIGNATURE"
elif [[ "$LOCAL_ERR" == "null" ]]; then
    echo "Local returned null. Your node may not have"
    echo "caught up to the slot containing this tx yet."
    echo "Wait and re-run with:"
    echo "  $0 $SIGNATURE"
elif [[ "$PUBLIC_ERR" == "null" ]]; then
    echo "Public returned null but local didn't."
    echo "Try a different public RPC endpoint:"
    echo "  PUBLIC_RPC=<url> $0 $SIGNATURE"
elif [[ "$LOCAL_ERR" == "$PUBLIC_ERR" ]]; then
    echo "SAME error codes. The patch may not be active,"
    echo "or this transaction didn't hit the patched"
    echo "code path."
else
    echo "DIFFERENT error codes!"
    echo "  Local:  $LOCAL_ERR"
    echo "  Public: $PUBLIC_ERR"
fi
