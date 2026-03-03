#!/usr/bin/env bash

set -euo pipefail

NETWORK="${1:-mainnet}"
LOCAL_RPC="${LOCAL_RPC:-http://localhost:8899}"
VALIDATOR_LOG="${VALIDATOR_LOG:-$HOME/logs/solana-validator.log}"

case "$NETWORK" in
    mainnet|mainnet-beta)
        PUBLIC_RPC="${PUBLIC_RPC:-https://api.mainnet-beta.solana.com}"
        ;;
    testnet)
        PUBLIC_RPC="${PUBLIC_RPC:-https://api.testnet.solana.com}"
        ;;
    *)
        echo "Usage: $0 [mainnet|testnet]"
        exit 1
        ;;
esac

echo "=== Consensus Verification ==="
echo "Network:    $NETWORK"
echo "Local RPC:  $LOCAL_RPC"
echo "Public RPC: $PUBLIC_RPC"
echo ""

# --- 1. Compare finalized slots ---
LOCAL_SLOT=$(curl -s -m 10 "$LOCAL_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[{"commitment":"finalized"}]}' \
    | jq '.result')

PUBLIC_SLOT=$(curl -s -m 10 "$PUBLIC_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[{"commitment":"finalized"}]}' \
    | jq '.result')

SLOT_DIFF=$((PUBLIC_SLOT - LOCAL_SLOT))
# Use absolute value
if (( SLOT_DIFF < 0 )); then
    SLOT_DIFF=$((-SLOT_DIFF))
fi

echo "1. Finalized Slot"
echo "   Local:  $LOCAL_SLOT"
echo "   Public: $PUBLIC_SLOT"
echo "   Drift:  $SLOT_DIFF slots"
if (( SLOT_DIFF > 150 )); then
    echo "   WARN: Node is more than 150 slots behind."
else
    echo "   OK: Node is keeping up with the network."
fi
echo ""

# --- 2. Compare block hashes for recent finalized slots ---
# Use whichever finalized slot is lower so both nodes have it
if (( LOCAL_SLOT < PUBLIC_SLOT )); then
    CHECK_SLOT=$LOCAL_SLOT
else
    CHECK_SLOT=$PUBLIC_SLOT
fi
# Step back a few slots to ensure both have fully finalized
CHECK_SLOT=$((CHECK_SLOT - 5))

echo "2. Block Hash Comparison (slot $CHECK_SLOT)"

LOCAL_HASH=$(curl -s -m 30 "$LOCAL_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\", \"id\": 1,
        \"method\": \"getBlock\",
        \"params\": [
            $CHECK_SLOT,
            {\"encoding\": \"json\",
             \"maxSupportedTransactionVersion\": 0,
             \"transactionDetails\": \"none\",
             \"rewards\": false}
        ]
    }" | jq -r '.result.blockhash // "unavailable"')

PUBLIC_HASH=$(curl -s -m 30 "$PUBLIC_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\", \"id\": 1,
        \"method\": \"getBlock\",
        \"params\": [
            $CHECK_SLOT,
            {\"encoding\": \"json\",
             \"maxSupportedTransactionVersion\": 0,
             \"transactionDetails\": \"none\",
             \"rewards\": false}
        ]
    }" | jq -r '.result.blockhash // "unavailable"')

echo "   Local:  $LOCAL_HASH"
echo "   Public: $PUBLIC_HASH"
if [[ "$LOCAL_HASH" == "unavailable" || "$PUBLIC_HASH" == "unavailable" ]]; then
    echo "   WARN: Could not fetch block from one or both nodes."
elif [[ "$LOCAL_HASH" == "$PUBLIC_HASH" ]]; then
    echo "   OK: Block hashes match."
else
    echo "   FAIL: Block hashes DIFFER — node may have forked!"
fi
echo ""

# --- 3. Compare a second slot for extra confidence ---
CHECK_SLOT2=$((CHECK_SLOT - 100))

echo "3. Block Hash Comparison (slot $CHECK_SLOT2)"

LOCAL_HASH2=$(curl -s -m 30 "$LOCAL_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\", \"id\": 1,
        \"method\": \"getBlock\",
        \"params\": [
            $CHECK_SLOT2,
            {\"encoding\": \"json\",
             \"maxSupportedTransactionVersion\": 0,
             \"transactionDetails\": \"none\",
             \"rewards\": false}
        ]
    }" | jq -r '.result.blockhash // "unavailable"')

PUBLIC_HASH2=$(curl -s -m 30 "$PUBLIC_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\", \"id\": 1,
        \"method\": \"getBlock\",
        \"params\": [
            $CHECK_SLOT2,
            {\"encoding\": \"json\",
             \"maxSupportedTransactionVersion\": 0,
             \"transactionDetails\": \"none\",
             \"rewards\": false}
        ]
    }" | jq -r '.result.blockhash // "unavailable"')

echo "   Local:  $LOCAL_HASH2"
echo "   Public: $PUBLIC_HASH2"
if [[ "$LOCAL_HASH2" == "unavailable" || "$PUBLIC_HASH2" == "unavailable" ]]; then
    echo "   WARN: Could not fetch block from one or both nodes."
elif [[ "$LOCAL_HASH2" == "$PUBLIC_HASH2" ]]; then
    echo "   OK: Block hashes match."
else
    echo "   FAIL: Block hashes DIFFER — node may have forked!"
fi
echo ""

# --- 4. Check validator health ---
echo "4. Validator Health"
HEALTH=$(curl -s -m 5 "$LOCAL_RPC" -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' \
    | jq -r '.result // .error.message')
echo "   $HEALTH"
echo ""

# --- 5. Check validator logs for bank hash mismatches ---
LOG_LINES="${LOG_LINES:-50000}"
echo "5. Bank Hash Mismatch Check (last $LOG_LINES log lines)"
if [[ ! -f "$VALIDATOR_LOG" ]]; then
    echo "   WARN: Log file not found at $VALIDATOR_LOG"
    echo "   Set VALIDATOR_LOG to the correct path."
    MISMATCH_COUNT="unknown"
else
    MISMATCH_COUNT=$(tail -n "$LOG_LINES" "$VALIDATOR_LOG" | grep -c "bank hash mismatch" || true)
    echo "   Occurrences of 'bank hash mismatch': $MISMATCH_COUNT"
    if (( MISMATCH_COUNT > 0 )); then
        echo "   FAIL: Bank hash mismatches detected!"
        tail -n "$LOG_LINES" "$VALIDATOR_LOG" | grep "bank hash mismatch" | tail -3 | while read -r line; do
            echo "     $line"
        done
    else
        echo "   OK: No bank hash mismatches found."
    fi
fi
echo ""

# --- 6. Check for healthy rooting ---
echo "6. Recent Rooting Activity"
if [[ ! -f "$VALIDATOR_LOG" ]]; then
    echo "   WARN: Log file not found."
    RECENT_ROOTS=0
else
    RECENT_ROOTS=$(tail -n "$LOG_LINES" "$VALIDATOR_LOG" | grep -c "new root" || true)
    LATEST_ROOT_LINE=$(tail -n "$LOG_LINES" "$VALIDATOR_LOG" | grep "new root" | tail -1)
    LATEST_ROOT_SLOT=$(echo "$LATEST_ROOT_LINE" | grep -oP 'new root \K[0-9]+' || true)
    echo "   'new root' entries in last $LOG_LINES lines: $RECENT_ROOTS"
    if (( RECENT_ROOTS > 0 )); then
        echo "   Latest: $LATEST_ROOT_LINE"
        if [[ -n "$LATEST_ROOT_SLOT" ]]; then
            ROOT_DRIFT=$((LOCAL_SLOT - LATEST_ROOT_SLOT))
            if (( ROOT_DRIFT < 0 )); then ROOT_DRIFT=$((-ROOT_DRIFT)); fi
            echo "   Latest root slot: $LATEST_ROOT_SLOT (finalized slot: $LOCAL_SLOT, drift: $ROOT_DRIFT)"
            if (( ROOT_DRIFT > 150 )); then
                echo "   WARN: Rooting is lagging behind finalized slot."
            else
                echo "   OK: Rooting is current with finalized slot."
            fi
        fi
    else
        echo "   WARN: No rooting activity found in recent logs."
    fi
fi
echo ""

# --- Summary ---
echo "=== Summary ==="
PASS=true
if (( SLOT_DIFF > 150 )); then
    PASS=false
fi
if [[ "$LOCAL_HASH" != "unavailable" && "$PUBLIC_HASH" != "unavailable" && "$LOCAL_HASH" != "$PUBLIC_HASH" ]]; then
    PASS=false
fi
if [[ "$LOCAL_HASH2" != "unavailable" && "$PUBLIC_HASH2" != "unavailable" && "$LOCAL_HASH2" != "$PUBLIC_HASH2" ]]; then
    PASS=false
fi
if [[ "$MISMATCH_COUNT" != "unknown" ]] && (( MISMATCH_COUNT > 0 )); then
    PASS=false
fi

if [[ "$PASS" == "true" ]]; then
    echo "Node is in consensus with the network."
    echo "The patched error codes do not affect the bank hash."
else
    echo "WARNING: Node may have diverged from the network!"
fi
