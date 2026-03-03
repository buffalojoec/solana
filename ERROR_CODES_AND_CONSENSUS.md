# Do Transaction Error Codes Affect Consensus?

Solana contributors and enthusiasts alike have long stated that "error codes
don't affect consensus", however, I'm not aware of anywhere this has been
empirically explained and tested.

This post aims to describe why error codes are not included in consensus and
test the theory definitively.

## What Goes Into the Bank Hash

The bank hash is computed in
[`Bank::hash_internal_state()`][hash_internal_state].
Here is what it hashes, in order:

[hash_internal_state]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/runtime/src/bank.rs#L4805-L4874

```rust
let mut hash = hashv(&[
    self.parent_hash.as_ref(),
    &self.signature_count().to_le_bytes(),
    self.last_blockhash().as_ref(),
]);

// Accounts lattice hash (all account state changes)
let accounts_lt_hash =
    &*self.accounts_lt_hash.lock().unwrap();
let lt_hash_bytes =
    bytemuck::must_cast_slice(&accounts_lt_hash.0.0);
hash = hashv(&[hash.as_ref(), lt_hash_bytes]);

// Optional hard fork data
if let Some(buf) =
    self.hard_forks...get_hash_data(slot, self.parent_slot())
{
    hash = hashv(&[hash.as_ref(), &buf]);
}
```

Let's examine each one.

**Parent hash.** Inherited from the parent slot. Not affected by error codes.

**Signature count.** This is a count of the total signatures from all
*processed* transactions — both successful and failed.
[Accumulated here][sig_count]:

[sig_count]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/runtime/src/bank.rs#L3633-L3638

```rust
if processing_result.was_processed() {
    // was_processed() == self.is_ok() on the outer
    // Result, meaning the tx was executed or fees-only
    // — regardless of whether execution itself
    // succeeded or failed.
    processed_counts.signature_count +=
        tx.signature_details()
            .num_transaction_signatures();
}
```

This is a scalar count. It does not encode *which* transactions failed or what
errors they produced. A transaction that fails with
`InstructionError::InvalidArgument` contributes the same `+1` (or `+N` for multiple
signatures) as one that fails with `InstructionError::Custom(42)`.

**Last blockhash.** The most recent blockhash from the blockhash queue. Not
affected by error codes.

**Accounts lattice hash.** This is the real key piece of the bank hash
consensus cares about. The [lattice hash][lt_hash] captures the hash of every
account that was *modified* in this slot. Each account is
[hashed with BLAKE3][account_hash] over its fields: `lamports`, `data`,
`executable`, `owner`, and `pubkey`.

[lt_hash]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/runtime/src/bank/accounts_lt_hash.rs#L27-L276
[account_hash]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/accounts-db/src/accounts_db.rs#L4357-L4401

The question then becomes: when a transaction fails, which account
modifications survive?

## What Survives a Failed Transaction

The answer lives in
[`collect_accounts_to_store()`][collect_accounts]:

[collect_accounts]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/runtime/src/account_saver.rs#L51-L102

```rust
match processed_tx {
    ProcessedTransaction::Executed(executed_tx) => {
        if executed_tx
            .execution_details
            .status
            .is_ok()
        {
            // SUCCESS: store all writable accounts
            collect_accounts_for_successful_tx(...);
        } else {
            // FAILURE: store ONLY rollback accounts
            collect_accounts_for_failed_tx(
                ...,
                &executed_tx
                    .loaded_transaction
                    .rollback_accounts,
            );
        }
    }
    ProcessedTransaction::FeesOnly(fees_only_tx) => {
        // LOAD FAILURE: store ONLY rollback accounts
        collect_accounts_for_failed_tx(
            ...,
            &fees_only_tx.rollback_accounts,
        );
    }
}
```

The branching logic is binary: `status.is_ok()` or not. The *specific* error
inside the `Err(...)` variant is never inspected. Both branches produce the
same account storage behavior regardless of which error code is present.

### Rollback Accounts

[`RollbackAccounts`][rollback_accounts] is a small enum that captures at most
two accounts:

[rollback_accounts]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/svm/src/rollback_accounts.rs#L9-L23

1. **The fee payer** — with fees already deducted.
2. **The nonce account** (if a durable nonce was used) — with the nonce already
   advanced.

These snapshots are [taken *before* execution][rollback_capture], during fee
validation. The fee is [deducted here][fee_deduction]:

[rollback_capture]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/svm/src/transaction_processor.rs#L731-L738
[fee_deduction]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/svm/src/account_loader.rs#L389-L391

```rust
payer_account
    .checked_sub_lamports(fee)
    .map_err(|_| {
        TransactionError::InsufficientFundsForFee
    })?;
```

Critically, the fee amount is computed from the transaction's compute budget
and priority fee — **not** from the error code. Whether the transaction later
fails with `InvalidArgument` or `Custom(999)`, the fee payer's post-deduction
lamport balance is identical.

## Where Error Codes Actually Go

Error codes are stored in two places, neither of which feeds
into the bank hash:

### 1. The Status Cache (in-memory)

In [`update_transaction_statuses()`][update_statuses], after commit:

[update_statuses]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/runtime/src/bank.rs#L3053-L3081

```rust
status_cache.insert(
    tx.recent_blockhash(),
    tx.message_hash(),
    self.slot(),
    processed_tx.status(), // contains the error
);
```

The status cache is used for two purposes:

- **Deduplication**: preventing the same transaction from being processed
  twice. The [dedup check][dedup_check] only looks at whether an entry *exists*
  (returns `Option<Slot>`), not at the stored error code.
- **RPC queries**: `getSignatureStatuses` and similar.

[dedup_check]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/runtime/src/bank/check_transactions.rs#L267-L277

The status cache *is* serialized into snapshots, but it is not hashed into the
bank hash. Its presence in snapshots is purely for bootstrapping the
deduplication window on new validators.

### 2. The Blockstore (persistent)

In the [`TransactionStatusService`][tx_status_service], transaction metadata
(including error codes, logs, inner instructions, return data) is written to
RocksDB alongside the ledger entries. To be clear: the ledger entries
themselves — the actual transactions — are consensus-critical, because they are
the input to replay. But the *metadata* written by `TransactionStatusService`
is a sidecar for historical RPC queries. It is not replayed, not hashed, and
has no bearing on the bank hash.

[tx_status_service]: https://github.com/anza-xyz/agave/blob/6f2fe18406c8ab16a5c30b99579cb202574bf48d/rpc/src/transaction_status_service.rs#L122-L256

## An Example

Suppose Validator A processes a transaction that calls a builtin program and
gets `InstructionError::InvalidArgument`. Validator B processes the same
transaction and gets `InstructionError::Custom(7)`. Both agree the transaction
failed.

The bank hash inputs for both validators are:

| Component | Node A | Node B | Match? |
|---|---|---|---|
| Parent hash | Same | Same | ✅ |
| Signature count | +1 | +1 | ✅ |
| Last blockhash | Same | Same | ✅ |
| Fee payer lamports | `bal - fee` | `bal - fee` | ✅ |
| Fee payer data | unchanged | unchanged | ✅ |
| Nonce account | advanced | advanced | ✅ |
| All other accounts | rolled back | rolled back | ✅ |
| **Bank hash** | **H** | **H** | **✅** |

The error codes `InvalidArgument` and `Custom(7)` are stored in each
validator's status cache and blockstore, but these are local metadata stores.
They never enter `hash_internal_state()`.

## The Real Status Boundary: Pass vs. Fail

While error codes don't matter, the pass/fail distinction absolutely does. If
Validator A thinks a transaction succeeded and Validator B thinks it failed,
they will store completely different account states:

- **Success**: all modified writable accounts are committed.
- **Failure**: only fee payer (fee-deducted) and nonce (advanced) are
  committed; everything else is rolled back.

This produces different accounts lattice hashes, different bank hashes, and a
consensus failure. The pass/fail boundary is sacred. The error code within the
failure is not.

Similarly, the distinction between "not processed at all" (fee not charged,
signature count not incremented) and "processed but failed" (fee charged,
signature count incremented) is also consensus-critical. But within the
"failed" category, the specific reason for failure is irrelevant to consensus.

## A Test

To confirm empirically, I patched a mainnet validator's System program to
return `InvalidArgument` instead of `InsufficientFunds` on failed transfers.
I also patched the CLI to override any balance checks.

You can check out my branch [here][patched_branch].

[patched_branch]: https://github.com/buffalojoec/solana/tree/error-codes-consensus-test

> Note: You will need to run a non-voting validator with all RPC features
> enabled, which degrades performance. Thus, the querying will only work for
> a very short time window. Once the transaction is buried in the ledger the
> node will not have enough resources to query the blockstore while running
> replay.

Simply invoke a transfer with a balance greater than the sender's balance:

```
./target/release/solana transfer \
  --allow-unfunded-recipient \
  --no-wait \
  --skip-preflight \
  <DESTINATION> \
  5000
```

Take the returned signature and inspect it on the Solana Explorer:

```
https://explorer.solana.com/tx/<SIGNATURE>
```

Then run the comparison script to see the outputs from your local node's RPC
versus the mainnet public RPC endpoint:

```
./error-codes-test/compare-error-codes.sh <SIGNATURE>
```

You should see an output like this, which shows two different error codes
returned for the same transaction:

```
=== Error Codes Consensus Test ===
Network:    mainnet
Local RPC:  http://localhost:8899
Public RPC: https://api.mainnet-beta.solana.com

Signature: 4MT4uNXL9PJrwtq9j6bM7kMihDNnVL2PvgMFSjSARyUX9KA66xdjEazUSAtv4Rhboo9Yj23qwf9Pui2MNw7cMxAt

Waiting for finalization (~15s)...

=== Local RPC (patched validator) ===
{
  "InstructionError": [
    0,
    "InvalidArgument"
  ]
}

=== Public RPC (stock validators) ===
{
  "InstructionError": [
    0,
    {
      "Custom": 1
    }
  ]
}
```

<details>
  <summary>If the script hangs click here</summary>

  You may have issues reading from the blockstore due to the resource
  constraints mentioned previously in this section.

  If the comparison script hangs and does not return the response for your
  local node, try piping everything all at once:

  ```
  <TRANSFER COMMAND> \
    | awk 'NF { sub(/^[[:space:]]*Signature:[[:space:]]*/, ""); print }' \
    | xargs ./error-codes-test/compare-error-codes.sh
  ```
</details>

Next run the consensus check script to ensure your local node hasn't forked
off from the network:

```
./error-codes-test/verify-consensus.sh
```

You should see an output like this, which shows that your node is still synced
and has not produced a bad bank hash:

```
=== Consensus Verification ===
Network:    mainnet
Local RPC:  http://localhost:8899
Public RPC: https://api.mainnet-beta.solana.com

1. Finalized Slot
   Local:  404105318
   Public: 404105318
   Drift:  0 slots
   OK: Node is keeping up with the network.

2. Block Hash Comparison (slot 404105313)
   Local:  X7tbu5mQq6DDGJ1nDnDp1V8KciAyVXq1dCVgvRRUjAp
   Public: X7tbu5mQq6DDGJ1nDnDp1V8KciAyVXq1dCVgvRRUjAp
   OK: Block hashes match.

3. Block Hash Comparison (slot 404105213)
   Local:  ESAcoqbTWPyhQFbGRXt23Cxh5hLgFHe5vGGZqmKgL3uC
   Public: ESAcoqbTWPyhQFbGRXt23Cxh5hLgFHe5vGGZqmKgL3uC
   OK: Block hashes match.

4. Validator Health
   ok

5. Bank Hash Mismatch Check (last 50000 log lines)
   Occurrences of 'bank hash mismatch': 0
   OK: No bank hash mismatches found.

6. Recent Rooting Activity
   'new root' entries in last 50000 lines: 674
   Latest: [2026-03-04T06:47:32.585473245Z INFO  agave_votor::root_utils] Gra75rjTdvggVzADDgX6vYaeWPNbNJ352JUJa6fyvh1f: new root 404105320
   Latest root slot: 404105320 (finalized slot: 404105318, drift: 2)
   OK: Rooting is current with finalized slot.

=== Summary ===
Node is in consensus with the network.
The patched error codes do not affect the bank hash.
```

---

**The claim is true.** Error codes do not affect consensus. The bank hash is a
function of account state, and account state for failed transactions depends
only on fee deduction and nonce advancement — neither of which varies with the
error code.
