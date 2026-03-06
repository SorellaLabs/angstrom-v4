---
stage: plan
pipeline: feature
timestamp: 2026-03-06T00:00:00Z
arguments: "Reduce RPC request volume in L2 swap tests to avoid QuickNode 429 rate limiting"
---

# Plan: Reduce RPC Request Volume in L2 Swap Tests

## Goal

Fix `test_l2_swap_with_mev_tax_matches_onchain` failing with HTTP 429 (50/second rate limit) by reducing the number of RPC calls during pool initialization.

## Root Cause Analysis

Each test creates a `PoolManagerServiceBuilder` with `tick_band=6000` and `ticks_per_batch=10` (default). During `BaselinePoolFactory::new()`, tick loading makes:

- **Per direction**: 6000 / 10 = 600 batch RPC calls
- **Both directions** (concurrent via `futures::join`): 1,200 calls per pool
- **Two tests** run sequentially: 2,400+ total calls

With a 50 req/s limit, the first test consumes the budget and the second test immediately 429s during initialization.

## Approach

Add `.with_ticks_per_batch(300)` to both tests. This reduces tick loading from 1,200 to **40 RPC calls per pool** (6000/300 = 20 per direction × 2). The `GetUniswapV4TickData` contract handles large batch sizes fine since it runs as `eth_call` with block gas limit.

**Why 300**: Conservative enough to avoid `eth_call` gas issues on any provider, while reducing calls by 30x. Even with 2 tests, total tick calls drop from ~2,400 to ~80.

## Options Considered

- **Increase ticks_per_batch (chosen)**: Test-only change, zero library impact, 30x reduction in RPC calls
- **Share pool state between tests**: Would halve calls but couples tests and complicates debugging
- **Provider-level rate limiter**: Most robust for production but adds dependency, overkill for test fix

## Implementation Steps

### Step 1: Add `.with_ticks_per_batch(300)` to `test_l2_swap_matches_onchain`

**File**: `crates/uni-v4/tests/l2_revm_swap_test.rs`
**Change**: Add `.with_ticks_per_batch(300)` to the builder chain (after `.with_auto_pool_creation(true)`, before `.with_current_block(TARGET_BLOCK)`)
**Lines**: ~119-131

```rust
let service = PoolManagerServiceBuilder::new_with_noop_stream(
    provider.clone(),
    L2AddressBook::new(ANGSTROM_L2_FACTORY),
    L2PoolRegistry::default(),
    POOL_MANAGER,
    DEPLOY_BLOCK
)
.with_initial_tick_range_size(6000)
.with_auto_pool_creation(true)
.with_ticks_per_batch(300)
.with_current_block(TARGET_BLOCK)
.build()
.await
.expect("Failed to create service");
```

**Verification**: `cargo test --test l2_revm_swap_test test_l2_swap_matches_onchain` passes without 429 errors

### Step 2: Add `.with_ticks_per_batch(300)` to `test_l2_swap_with_mev_tax_matches_onchain`

**File**: `crates/uni-v4/tests/l2_revm_swap_test.rs`
**Change**: Same builder modification at the second test (~lines 253-265)

```rust
let service = PoolManagerServiceBuilder::new_with_noop_stream(
    provider.clone(),
    L2AddressBook::new(ANGSTROM_L2_FACTORY),
    L2PoolRegistry::default(),
    POOL_MANAGER,
    DEPLOY_BLOCK
)
.with_initial_tick_range_size(6000)
.with_auto_pool_creation(true)
.with_ticks_per_batch(300)
.with_current_block(TARGET_BLOCK)
.build()
.await
.expect("Failed to create service");
```

**Verification**: `cargo test --test l2_revm_swap_test` — both tests pass

**Dependencies**: None (independent of Step 1, but both should be applied)

## Testing Plan

1. Run both tests together: `cargo test --test l2_revm_swap_test -- --test-threads=1`
2. Verify no 429 errors in output
3. Verify swap delta assertions still pass (same pool data, just loaded in fewer batches)

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Large batch hits `eth_call` gas limit | Low — `eth_call` uses block gas limit (~30M on Base), 300 ticks is well within | Reduce to 100 if any provider rejects |
| Tick data differs with larger batches | None — contract returns same data regardless of batch size | N/A |

## Follow-up Work

- Consider setting a sensible `DEFAULT_TICKS_PER_BATCH` higher than 10 in the library itself (currently in `baseline_pool_factory.rs:25`) to benefit all users, not just tests
- For production use with strict rate limits, a provider-level rate limiter could be added later
