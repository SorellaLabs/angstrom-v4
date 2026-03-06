---
stage: execute
pipeline: bug
timestamp: 2026-03-06T19:00:00Z
arguments: "upgrade-test-contracts"
---

# Execute: Upgrade SwapQuoter Test Contract

## Files Modified

1. **`contracts/src/test/SwapQuoter.sol`** — Added selector validation in `quote()` catch clause
2. **`crates/uni-v4/tests/l1_revm_swap_test.rs`** — Updated embedded bytecode + added `error UnexpectedRevert`
3. **`crates/uni-v4/tests/l2_revm_swap_test.rs`** — Updated embedded bytecode + added `error UnexpectedRevert`

## Changes Summary

### SwapQuoter.sol
- Added `error UnexpectedRevert(bytes reason)` custom error
- Added selector extraction (`bytes4 selector` from memory position 32)
- Added validation: `selector != QuoteResult.selector || reason.length < 68` → reverts with `UnexpectedRevert(reason)`
- Only decodes `amount0`/`amount1` after confirming the revert is a `QuoteResult`

### Rust Test Files (both L1 and L2)
- Replaced old bytecode (0x60a0...07cc) with newly compiled bytecode (0x60a0...0813)
- Added `error UnexpectedRevert(bytes reason)` to the `sol!` ABI block

## Verification

- `forge build` — compiled successfully (Solc 0.8.26)
- `cargo check --package uni-v4 --test l1_revm_swap_test --test l2_revm_swap_test` — passed

## Deviations from Plan

None — implementation matches the plan exactly.

## Follow-up

Integration tests require `ETH_URL` / `BASE_URL` env vars pointing to archive nodes. The contract fix will surface real errors (e.g., `HookDeltaExceedsSwapAmount`, `CannotSwapWhileLocked`) instead of returning garbage values when swaps fail for non-quote reasons.
