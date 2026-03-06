---
stage: plan
pipeline: bug
timestamp: 2026-03-05T00:00:00Z
arguments: "out of initialized tick ranges loaded for uniswap on large L2 swap"
---

# Bug Fix Plan: Out of Initialized Tick Ranges

## Issue Summary

The test `test_l2_swap_matches_onchain` passes for the small CBBTC->ETH swap (100K sats) but panics with "out of initialized tick ranges loaded for uniswap" on the large swap (10M sats). On-chain V4 never has this issue because it has all tick data. The local sim should match 1:1.

## Root Cause

**The tick batch loading advancement is wrong.** There's a unit mismatch between how the Solidity contract scans ticks and how the Rust code advances between batches.

### The Solidity contract (`GetUniswapV4TickData.sol`)

The contract calls `getNextTickGt`/`getNextTickLe` which walks the tick bitmap one **bitmap position** at a time. Each bitmap position = `tick_spacing` tick units (60 for this pool). So `numTicks=10` scans 10 bitmap positions = 10 * 60 = 600 tick units.

After each iteration, the contract advances `currentTick` to the found tick:
```solidity
currentTick = nextTick;
if (zeroForOne) { --currentTick; }
```

### The Rust batch advancement (`baseline_pool_factory.rs:327-332`)

```rust
let next_tick = if zero_for_one {
    tick_start.as_i32() - (num_ticks as i32)     // -10 raw ticks
} else {
    tick_start.as_i32() + (num_ticks as i32)     // +10 raw ticks
};
```

This advances by `num_ticks` **raw tick units** (10), not `num_ticks * tick_spacing` **bitmap positions** (600). Then the caller adds one more `tick_spacing`:

```rust
tick_start = next_tick + tick_spacing;  // +70 total per batch
```

### Impact on coverage

- **Expected**: With `tick_band=6000` and `tick_spacing=60`, coverage should be `6000 * 60 = 360,000` tick units per direction
- **Actual**: Each batch advances only 70 raw tick units (`10 + tick_spacing`). With 600 batches: `600 * 70 = 42,000` tick units per direction — **only ~12% of expected coverage**
- **Massive redundant fetching**: The contract scans 600 tick units per batch, but Rust restarts only 70 units later, re-fetching ~88% of the same ticks each time

The 42,000 tick range is insufficient for the 0.1 cbBTC swap, causing the "out of initialized tick ranges" error.

## Proposed Fix

### Change 1: Fix batch advancement to use actual contract response (`baseline_pool_factory.rs`)

The `TickData` array returned by the contract contains the actual tick positions scanned. The last valid entry's `.tick` field is the farthest position the contract reached. Use that instead of the arithmetic `tick_start + num_ticks`.

**File: `crates/uni-v4-upkeeper/src/baseline_pool_factory.rs`**

#### Fix `get_tick_data_batch_request` (line 327-332):

```rust
// BEFORE (wrong - advances by raw tick units):
let next_tick = if zero_for_one {
    tick_start.as_i32() - (num_ticks as i32)
} else {
    tick_start.as_i32() + (num_ticks as i32)
};

// AFTER (correct - use actual last tick from contract):
let next_tick = ticks
    .last()
    .map(|t| t.tick.as_i32())
    .unwrap_or_else(|| {
        if zero_for_one {
            tick_start.as_i32() - (num_ticks as i32 * tick_spacing)
        } else {
            tick_start.as_i32() + (num_ticks as i32 * tick_spacing)
        }
    });
```

The fallback (when batch returns no ticks) uses `num_ticks * tick_spacing` which is the correct advancement if the area has no ticks at all.

#### Fix `get_tick_data_batch_request_static` (line 500-505) — same change:

```rust
let next_tick = ticks
    .last()
    .map(|t| t.tick.as_i32())
    .unwrap_or_else(|| {
        if zero_for_one {
            tick_start.as_i32() - (num_ticks as i32 * tick_spacing)
        } else {
            tick_start.as_i32() + (num_ticks as i32 * tick_spacing)
        }
    });
```

#### Fix `request_more_ticks` (line 618-624) — same pattern:

The `request_more_ticks` method has inline batch advancement with the same bug:
```rust
// BEFORE:
tick_start = if zero_for_one {
    tick_start - (ticks_to_load as i32 * tick_spacing)
} else {
    tick_start + (ticks_to_load as i32 * tick_spacing)
};
```
This one actually multiplies by `tick_spacing` already — it's correct! Only `get_tick_data_batch_request` and `get_tick_data_batch_request_static` have the bug.

### No changes needed to `liquidity_base.rs`

The boundary check at line 387 is correct — it prevents incorrect results from missing tick data. With the batching fix, the loaded range will be ~8.5x larger (360K vs 42K tick units), which should be sufficient.

### No changes needed to the test

The test already uses `with_initial_tick_range_size(6000)` which, with the fix, will provide 360K tick units of coverage per direction. This is more than enough for a 0.1 cbBTC swap.

## Why This Approach

- **Fixes root cause**: The advancement bug is the reason 6000 ticks worth of loading only covers 42K tick units instead of 360K
- **Minimal change**: Two function fixes (plus their static duplicates)
- **Improves all pools**: Not test-specific — fixes tick loading for production too
- **Reduces RPC calls**: Eliminates ~88% redundant re-fetching of the same ticks
- **Matches on-chain**: With correct coverage, swaps that succeed on-chain will also succeed locally

## Caveats

- **The `request_more_ticks` method (line 618)** already uses `ticks_to_load * tick_spacing` for advancement — it doesn't have this bug. Only the batch request functions do.
- After this fix, with `tick_band=6000` and `tick_spacing=60`, coverage becomes 360K tick units per direction. If a pool has extreme liquidity gaps requiring even more coverage, `tick_band` may need increasing — but that's a configuration issue, not a code bug.

## Verification

Run the integration test:
```bash
cargo test --package uni-v4 --test l2_revm_swap_test -- test_l2_swap_matches_onchain --nocapture
```

Both small and large swaps in both directions should pass with exact delta matches.

No open questions — the plan is complete and ready for execution.
