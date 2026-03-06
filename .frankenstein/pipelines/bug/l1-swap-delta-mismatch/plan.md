---
stage: plan
pipeline: bug
timestamp: 2026-03-06T00:00:00Z
arguments: "test_l1_swap_replay_matches_onchain token0 delta mismatch"
---

# Plan: Fix L1 Swap Delta Mismatch

## Issue Summary

The `test_l1_swap_replay_matches_onchain` test fails because the Angstrom hook is **not properly unlocked** before the on-chain swap quote. The test sets `_lastBlockUpdated = current_block + 1`, but `eth_call` runs with `block.number = current_block`, so `_isUnlocked()` returns `false`. The hook reverts with `CannotSwapWhileLocked()`, and the SwapQuoter misinterprets the V4 `WrappedError` revert data as swap deltas, producing garbage values.

## Root Cause

Off-by-one error on `l1_revm_swap_test.rs:311`:
```rust
let new_val = (slot_val & !mask) | U256::from(current_block + 1);  // +1 is wrong
```

## Proposed Fix

### File: `crates/uni-v4/tests/l1_revm_swap_test.rs`

**Change 1** — Remove the `+ 1` offset (line 311):

```rust
// BEFORE:
let new_val = (slot_val & !mask) | U256::from(current_block + 1);

// AFTER:
let new_val = (slot_val & !mask) | U256::from(current_block);
```

**Reasoning**: `get_block_number()` returns the latest mined block number `N`. The `eth_call` executes with `block.number = N` (standard "latest" semantics). Setting `_lastBlockUpdated = N` makes `_isUnlocked()` return `true` since `N == N`.

The deploy transaction (SwapQuoter) mines block `FINAL_BLOCK + 1`, so `current_block = FINAL_BLOCK + 1`. No further blocks are mined by `anvil_set_storage_at` (cheat code, no mining) or `.call()` (read-only). The value is stable across the entire loop.

### Why only one approach

- The `+ 1` is a clear off-by-one error — there's no alternative interpretation that makes the test pass
- The storage slot (3), bit layout (low 64 bits = `_lastBlockUpdated`), and mask logic are all correct
- The SwapQuoter bytecode is test infrastructure; fixing the quoter's error handling is desirable but not necessary — the root cause is the wrong unlock value

### What this does NOT change

- `pool_swap.rs` — the local swap simulation is correct
- The SwapQuoter contract bytecode — while it violates "never silently default on failure paths" (it returns garbage on unexpected reverts), fixing the unlock is sufficient
- The L2 test (`l2_revm_swap_test.rs`) — does not use hook unlocking

## Verification

After applying the fix, run:
```bash
cargo test --package uni-v4 --test l1_revm_swap_test -- test_l1_swap_replay_matches_onchain --exact --nocapture
```

Expected: all swap comparisons pass with matching deltas (local vs on-chain).

## Caveats

1. **Anvil `eth_call` block semantics**: If Anvil's `eth_call` actually uses `block.number = latest + 1` (pending block semantics), the fix would need to be `current_block + 1` and the original code was correct for a different reason. In that case, the bug would be elsewhere. However, the test IS currently failing with `+ 1`, which rules this out.

2. **Potential secondary issue**: Once the hook is properly unlocked, the `afterSwap` protocol fee may still cause small mismatches if the local `fee_config.protocol_fee()` value doesn't exactly match on-chain `_unlockedFees[key].protocolUnlockedFee`. This would manifest as a proportional difference (not 10^23x), and would be a separate bug to investigate if it appears.

## Follow-up Work (optional, not blocking)

- Consider adding revert selector validation to the SwapQuoter contract so hook-locked errors surface as clear panics rather than garbage data
- Consider adding a sanity check in `assert_deltas_match` to catch obviously invalid values (e.g., delta magnitude > total supply of the token)
