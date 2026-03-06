---
stage: plan
pipeline: bug
timestamp: 2026-03-05T00:00:00Z
arguments: "L2 swap fee mismatch — local sim uses swap_fee=0 but on-chain V4 applies pool's static LP fee"
---

# Bug Fix Plan: L2 Swap Fee Mismatch

## Issue Summary

The local swap simulation produces ~0.016% more output than the on-chain V4 quoter for L2 pools. The root cause: `L2FeeConfiguration::swap_fee()` returns `0`, but on-chain V4 applies the pool's static LP fee (e.g., 160 = 0.016%) via `Pool.sol`'s `slot0.lpFee()`.

The AngstromL2 hook returns 0 as the LP fee override, but since the pool uses a static fee (fee=160, not 0x800000 dynamic), V4 ignores the hook's return value entirely (`Hooks.sol:263` only parses fee override for dynamic fee pools). The AMM always charges the LP fee from the pool's `PoolKey.fee`.

## Root Cause

The `hook_fee` value (pool's LP fee from `PoolKey.fee`) is already captured in `L2PoolUpdate::NewPool` and used to construct the `PoolKey`, but it's **never stored in `L2FeeConfiguration`**. So when `PoolSwap` calls `self.fee_config.swap_fee()`, it gets 0 instead of the pool's actual LP fee.

## Proposed Fix (Single approach — surgical)

### Files to modify

#### 1. `crates/uni-v4-structure/src/fee_config.rs`

Add `lp_fee: u32` to `L2FeeConfiguration` and return it from `swap_fee()`:

```rust
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct L2FeeConfiguration {
    pub is_initialized:         bool,
    pub lp_fee:                 u32,  // NEW: Pool's LP fee from PoolKey.fee
    pub creator_tax_fee_e6:     u32,
    pub protocol_tax_fee_e6:    u32,
    pub creator_swap_fee_e6:    u32,
    pub protocol_swap_fee_e6:   u32,
    pub priority_fee_tax_floor: u128,
    pub jit_tax_enabled:        bool,
    pub withdraw_only:          bool
}
```

Update the `FeeConfig` impl:

```rust
impl FeeConfig for L2FeeConfiguration {
    // ...
    fn swap_fee(&self) -> u32 {
        self.lp_fee  // was: 0
    }
    // ...
}
```

Update the doc comment on `FeeConfig::swap_fee()` trait (line 39):
```rust
/// Returns the swap fee applied during the swap (in compute_swap_step).
/// - L1: LP fee charged during swap
/// - L2: Pool's static LP fee from PoolKey.fee (charged by V4 AMM)
fn swap_fee(&self) -> u32;
```

Update the doc comment on `FeeConfig::fee()` (line 51):
```rust
/// - L2: lp_fee + protocol_fee (creator + protocol swap fees)
fn fee(&self, bundle: bool) -> u32;
```

Update the `l2_fee_config()` test helper to include the new field:
```rust
fn l2_fee_config(floor: u128) -> L2FeeConfiguration {
    L2FeeConfiguration {
        is_initialized:         true,
        lp_fee:                 0,  // Most tests don't need LP fee
        creator_tax_fee_e6:     1000,
        // ... rest unchanged
    }
}
```

#### 2. `crates/uni-v4-upkeeper/src/l2/pool_manager_service.rs` (line 53)

Pass `hook_fee` as `lp_fee` when constructing `L2FeeConfiguration`:

```rust
let fee_cfg = L2FeeConfiguration {
    is_initialized:         true,
    lp_fee:                 *hook_fee,  // NEW
    creator_tax_fee_e6:     *creator_tax_fee_e6,
    // ... rest unchanged
};
```

#### 3. `crates/uni-v4-upkeeper/src/l2/update_provider.rs`

Two construction sites need updating:

**Line ~498** (in `fetch_l2_pools`, the `chain_updates.for_each` match arm):
```rust
fee_cfg: L2FeeConfiguration {
    is_initialized: true,
    lp_fee: hook_fee,  // NEW
    creator_tax_fee_e6,
    // ... rest unchanged
}
```

**Line ~53** (in `process_l2_factory_logs`, the `PoolCreated` handler — but this is actually in pool_manager_service.rs which we already cover above. The `process_l2_factory_logs` creates `L2PoolUpdate::NewPool` which already carries `hook_fee` — it flows through to `pool_manager_service.rs` where `L2FeeConfiguration` is constructed.)

### No changes needed to `pool_swap.rs`

The swap code at line 127 already does:
```rust
let swap_fee = if self.is_bundle { 0 } else { self.fee_config.swap_fee() };
```

Once `swap_fee()` returns the pool's LP fee instead of 0, this will work correctly. Bundle mode still uses 0 (correct — bundles bypass LP fees).

### Comment update in `pool_swap.rs` (line 61)

The comment says "AMM runs with reduced input and LP fee = 0" — update to:
```rust
//   - AMM runs with reduced input and pool's LP fee
```

## Why This Approach

- **Minimal change**: Only adds one field to a struct and threads an already-available value through construction sites
- **Correct by construction**: The `hook_fee` value already exists in `L2PoolUpdate::NewPool` and is derived from `PoolKey.fee` — the on-chain source of truth
- **No behavioral change for L1**: L1 pools use `L1FeeConfiguration` which is unaffected
- **Bundle mode unaffected**: Bundle swaps already use `swap_fee = 0` regardless of fee_config

## Caveats

- **Serialization**: Adding `lp_fee` to `L2FeeConfiguration` changes its serialized form (it derives `Serialize`/`Deserialize`). Any persisted state using this struct will need migration or reinitialization. Check if any caching/persistence exists.
- **The `pool_swap.rs` comment on line 53-61** describes the L2 BeforeSwapDelta flow. The comment's claim that "LP fee = 0" should be corrected to reflect the pool's actual LP fee.

## Verification

The existing integration test `test_l2_swap_matches_onchain` (in `crates/uni-v4/tests/l2_revm_swap_test.rs`) already validates local vs on-chain results. After the fix, the ~0.016% discrepancy should disappear and the `assert_eq!` on deltas should pass.

No open questions — the plan is complete and ready for execution.
