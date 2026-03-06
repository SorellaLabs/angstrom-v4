---
stage: execute
pipeline: bug
timestamp: 2026-03-05T00:00:00Z
arguments: "Apply recommended fix from plan"
---

# Execute: L2 Swap Fee Mismatch Fix

## Files Modified

1. **`crates/uni-v4-structure/src/fee_config.rs`**
   - Added `lp_fee: u32` field to `L2FeeConfiguration`
   - Changed `swap_fee()` to return `self.lp_fee` instead of `0`
   - Updated doc comments on `swap_fee()` and `fee()` traits
   - Updated test helper `l2_fee_config()` with `lp_fee: 0`

2. **`crates/uni-v4-upkeeper/src/l2/pool_manager_service.rs`**
   - Added `lp_fee: *hook_fee` to `L2FeeConfiguration` construction in `handle_chain_specific_update`

3. **`crates/uni-v4-upkeeper/src/l2/update_provider.rs`**
   - Added `lp_fee: hook_fee` to `L2FeeConfiguration` construction in `fetch_l2_pools`

4. **`crates/uni-v4-structure/src/pool_swap.rs`**
   - Updated comment: "LP fee = 0" → "pool's LP fee"

## Verification

- `cargo build` — clean
- `cargo test -p uni-v4-structure` — 9/9 passed
- `cargo clippy` — clean

## Deviations from Plan

None. Implementation matched the plan exactly.

## Follow-up

- Run integration test `test_l2_swap_matches_onchain` with `BASE_URL` set to verify on-chain match
- Check if any serialized `L2FeeConfiguration` state exists that needs reinitialization
