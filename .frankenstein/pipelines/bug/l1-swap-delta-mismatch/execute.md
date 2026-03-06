---
stage: execute
pipeline: bug
timestamp: 2026-03-06T12:00:00Z
arguments: "Write(.frankenstein/pipelines/bug/l1-swap-delta-mismatch/plan.md)"
---

# Execute: Fix L1 Swap Delta Mismatch

## Files Modified

1. `crates/uni-v4/tests/l1_revm_swap_test.rs` (line 311)

## Changes Made

Removed the off-by-one `+ 1` when setting `_lastBlockUpdated` in the Angstrom hook unlock logic:

```rust
// BEFORE:
let new_val = (slot_val & !mask) | U256::from(current_block + 1);

// AFTER:
let new_val = (slot_val & !mask) | U256::from(current_block);
```

This ensures `_lastBlockUpdated` equals `block.number` during `eth_call`, making `_isUnlocked()` return `true` so the hook allows swaps instead of reverting with `CannotSwapWhileLocked()`.

## Verification

- `cargo check --package uni-v4 --test l1_revm_swap_test` — compiles successfully
- Full test run requires `ETH_URL` env var pointing to an Ethereum mainnet RPC (not run here)

## Deviations from Plan

None. The fix was applied exactly as proposed.

## Follow-up Work

- The SwapQuoter contract still silently returns garbage on unexpected reverts (no revert selector validation). This is a robustness issue but not blocking.
- Once the test passes, monitor for potential secondary mismatches from `afterSwap` protocol fees.
