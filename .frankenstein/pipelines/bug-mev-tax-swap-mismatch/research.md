---
stage: research
pipeline: bug
timestamp: 2026-03-06T00:00:00Z
arguments: "MEV tax swap test returns garbage on-chain values for small swap amounts"
---

# Bug Research: MEV Tax Swap Mismatch

## Summary

`test_l2_swap_with_mev_tax_matches_onchain` fails because the on-chain SwapQuoter returns garbage values when the AngstromL2 hook's MEV tax or fee deductions cause the V4 swap to revert with a non-`QuoteResult` error. The quoter's catch clause blindly decodes revert data at fixed offsets, producing nonsensical amounts (a0=1.63e38, a1=0) when the revert format doesn't match the expected `QuoteResult(int128, int128)`.

## Reproduction

- Test: `test_l2_swap_with_mev_tax_matches_onchain` in `crates/uni-v4/tests/l2_revm_swap_test.rs`
- Parameters: CBBTC->ETH (zero_for_one=false), amount=10000 sats (0.0001 CBBTC), gas_price=basefee+1gwei
- Local result: t0=0, t1=10000
- On-chain result: a0=163133741161778951440612350553538848207, a1=0

## Root Cause

Two interacting issues:

### Issue 1: SwapQuoter doesn't discriminate revert types

`contracts/src/test/SwapQuoter.sol:26-32` — The `quote()` function catches ALL reverts and reads `amount0` and `amount1` from fixed memory offsets (reason+36 and reason+68), assuming the revert is always a `QuoteResult(int128, int128)`:

```solidity
catch (bytes memory reason) {
    assembly {
        amount0 := mload(add(reason, 36))  // offset 0x24
        amount1 := mload(add(reason, 68))  // offset 0x44
    }
}
```

If the actual revert is a 4-byte selector-only error (e.g., `HookDeltaExceedsSwapAmount`), the reason data is only 4 bytes. Reading at offsets 36 and 68 returns whatever was previously in memory — garbage.

### Issue 2: MEV tax overwhelms small swap amounts

The MEV tax for 1 gwei priority fee is `99 * 120_000 * 1e9 = 11,880,000,000,000,000 wei` (0.01188 ETH). For a 10000 sat CBBTC swap (~$10), the ETH output from the AMM is negligible compared to the MEV tax.

In V4's `Hooks.sol:272-278`, if the hook's `specifiedDelta` flips the sign of `amountToSwap`, the swap reverts with `HookDeltaExceedsSwapAmount`. Without the AngstromL2 Solidity source, the exact trigger cannot be confirmed, but the garbage on-chain values are consistent with a non-QuoteResult revert being misinterpreted.

The local simulation handles this differently using `saturating_sub` (`pool_swap.rs:188`), producing `t0=0` when MEV tax exceeds the output, rather than reverting.

### Why the non-mev-tax test passes

`test_l2_swap_matches_onchain` uses `gas_price=basefee` → priority_fee=0 → mev_tax=0. The hook returns zero BeforeSwapDelta, the swap proceeds normally, and the QuoteResult revert contains valid amounts.

## Code Analysis

### SwapQuoter vulnerability (`contracts/src/test/SwapQuoter.sol:26-32`)

The quoter assumes `manager.swap()` always succeeds (producing a valid `QuoteResult` revert from `unlockCallback`). But if the swap itself reverts (hook error, overflow, etc.), the catch clause misinterprets the error data.

### Local MEV tax application (`crates/uni-v4-structure/src/pool_swap.rs:57-93`)

For CBBTC->ETH (zero_for_one=false, ether_is_input=false):
- `before_swap_input_deduction` = protocol fee on CBBTC input (small)
- `before_swap_output_deduction` = MEV tax on ETH output (large: 11.88e15 wei)
- After AMM swap, `final_d_t0 = total_d_t0.saturating_sub(mev_tax)` → 0 for small swaps
- `final_d_t1 = total_d_t1 + fee` → ~10000

### V4 HookDeltaExceedsSwapAmount check (`Hooks.sol:272-278`)

```solidity
if (hookDeltaSpecified != 0) {
    bool exactInput = amountToSwap < 0;
    amountToSwap += hookDeltaSpecified;
    if (exactInput ? amountToSwap > 0 : amountToSwap < 0) {
        HookDeltaExceedsSwapAmount.selector.revertWith();
    }
}
```

This reverts with a 4-byte selector (no parameters). The quoter reads at offsets 36 and 68, well past the 4-byte data.

## External Knowledge

- AngstromL2.sol source is NOT in this repository. It's referenced in comments but deployed on-chain at the hook address in the pool's PoolKey.
- V4's BeforeSwapDelta (`contracts/lib/v4-periphery/lib/v4-core/src/types/BeforeSwapDelta.sol`) packs specified (upper 128 bits) and unspecified (lower 128 bits) deltas into int256.
- Prior bug fix (commit `0d2bef2`): Restructured fee application to use BeforeSwapDelta model matching AngstromL2.sol. The fix was for a different issue (LP fee not being applied) but same code area.

## Impact Assessment

- **Direct**: `test_l2_swap_with_mev_tax_matches_onchain` always fails for small swap amounts where MEV tax dominates
- **Scope**: Test-only issue. The local simulation logic in `pool_swap.rs` may or may not match on-chain for the MEV tax edge case (can't verify without AngstromL2 source).
- **Production risk**: If the local sim's MEV tax application differs from on-chain (e.g., saturating_sub vs revert), quote estimates for small swaps with priority fees would be wrong.

## Related Areas

- `crates/uni-v4-structure/src/pool_swap.rs:176-193` — L2 final delta adjustment with saturating_sub
- `crates/uni-v4-structure/src/fee_config.rs:152-159` — MEV tax calculation
- `contracts/src/test/SwapQuoter.sol` — Quoter revert handling
- All other tests using SwapQuoter with MEV-taxed hooks would have the same garbage-decoding issue

## Relevant History

- `0d2bef2` — "fix: correct L2 swap fee application to match AngstromL2 beforeSwap logic" — introduced the current BeforeSwapDelta model
- `426d576` — "fix: separate L1 and L2 fee logic in pool swap to prevent cross-contamination"
- `3ea4ea2` — "fix: store pool LP fee in L2FeeConfiguration so local swap sim applies it"
- `b6fc950` — "fix: set gas_price=basefee on quoter calls to prevent uint256 underflow"

## Recommended Fix Approaches

1. **Simple (test-only)**: Skip small swap amounts in the mev_tax test, or use larger amounts where MEV tax < swap output. This avoids the edge case entirely.
2. **Better (quoter fix)**: Update SwapQuoter to verify the revert selector matches `QuoteResult` before decoding, and propagate unexpected errors.
3. **Best (full alignment)**: Fix the quoter AND verify the local sim's MEV tax handling matches on-chain behavior for all cases, including when mev_tax > output. This requires access to AngstromL2.sol source.
