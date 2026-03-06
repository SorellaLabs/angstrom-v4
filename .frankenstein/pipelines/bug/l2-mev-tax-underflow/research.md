---
stage: research
pipeline: bug
timestamp: 2026-03-06T19:20:00Z
arguments: "test_l2_swap_with_mev_tax_matches_onchain panics with UnexpectedRevert containing Panic(0x11) arithmetic underflow"
---

# Research: L2 MEV Tax Underflow on Small Swap Amounts

## Summary

The `test_l2_swap_with_mev_tax_matches_onchain` test fails on the very first swap (`CBBTC->ETH 0.0001 cbBTC`) because the on-chain Angstrom L2 hook's `beforeSwap` function panics with `Panic(0x11)` (arithmetic underflow). The MEV tax (0.01188 ETH) vastly exceeds the swap output (~0.0033 ETH), causing an unchecked subtraction to underflow in Solidity 0.8's checked arithmetic. The local Rust simulation uses `saturating_sub` and doesn't panic, creating a divergence between local and on-chain behavior.

## Reproduction

- Test: `test_l2_swap_with_mev_tax_matches_onchain` in `crates/uni-v4/tests/l2_revm_swap_test.rs`
- Trigger: First swap iteration — `CBBTC->ETH 0.0001 cbBTC` with `priority_fee = 1 gwei`
- Environment: Anvil fork of Base at block 42977290

## Root Cause

The test hardcodes `priority_fee = 1_000_000_000` (1 gwei) at line 306. With `basefee = 4,609,845` wei (~0.0046 gwei), this priority fee is 217x the basefee — unrealistically high for Base L2.

The MEV tax formula (`fee_config.rs:152-158`):
```
mev_tax = L2_SWAP_MEV_TAX_FACTOR * L2_SWAP_TAXED_GAS * (priority_fee - floor)
        = 99 * 120,000 * 1,000,000,000
        = 11,880,000,000,000,000 wei  (≈0.01188 ETH)
```

For the `CBBTC->ETH` direction (`zero_for_one = false`), ETH is the **output** token. The hook attempts to subtract the MEV tax from the swap's ETH output via a `BeforeSwapDelta`. Since 0.0001 cbBTC produces only ~0.0033 ETH output (~3.3×10¹⁵ wei), the subtraction `output - mev_tax` underflows.

**On-chain** (Solidity 0.8): The hook's `beforeSwap` uses checked arithmetic → `Panic(0x11)`.
**Local** (Rust): `pool_swap.rs:188` uses `saturating_sub` → clamps to 0, no panic.

## Code Analysis

### Error Decode

The revert data decodes to:
```
UnexpectedRevert(bytes)            // 0xc47bc67c — our new SwapQuoter error
  └─ WrappedError(                 // 0x90bfb865 — V4 PoolManager wrapper
       hook: 0xf96510247aba6a6b997b60ac4d98bb51aff265cf,
       selector: 0xb47b2fb1,       // beforeSwap hook function
       innerError: Panic(0x11),     // arithmetic underflow
       hookReturnData: 0xa9e35b2f   // beforeSwap return selector
     )
```

### Test code (`l2_revm_swap_test.rs:306-307`)
```rust
let priority_fee: u128 = 1_000_000_000; // 1 gwei
let gas_price = basefee as u128 + priority_fee;
```

### Local MEV tax handling (`pool_swap.rs:186-189`)
```rust
// oneForZero: token1 is input, token0 is output
let adj_t1 = total_d_t1.saturating_add(before_swap_input_deduction);
let adj_t0 = total_d_t0.saturating_sub(before_swap_output_deduction);  // ← saturating!
```

### MEV tax calculation (`fee_config.rs:152-159`)
```rust
fn mev_tax(&self, priority_fee_wei: u128) -> u128 {
    if priority_fee_wei <= self.priority_fee_tax_floor { return 0; }
    L2_SWAP_MEV_TAX_FACTOR * L2_SWAP_TAXED_GAS * (priority_fee_wei - self.priority_fee_tax_floor)
}
```

## Impact Assessment

- **Direct**: Only affects `test_l2_swap_with_mev_tax_matches_onchain` — the MEV tax test fails on every run at the first small swap amount.
- **No production impact**: This is a test-only issue. The on-chain hook correctly rejects swaps where the MEV tax exceeds the output (it *should* revert — there's no economic sense in executing such a swap).
- **The non-MEV-tax test (`test_l2_swap_matches_onchain`) is unaffected** — it uses `gas_price = basefee` (no priority fee, no MEV tax).

## Related Areas

- The local Rust MEV tax logic (`pool_swap.rs:56-93`) uses `saturating_sub`, which silently produces wrong results when mev_tax > output. This is a secondary concern — if the on-chain hook reverts, the local code should also detect and reject these swaps rather than returning a "successful" result with clamped-to-zero output.

## Relevant History

- The MEV tax test was added as part of the `feature-l1-integration-test-replay` pipeline.
- The `priority_fee = 1 gwei` was likely chosen as a round number without accounting for Base L2's extremely low basefee.
- The SwapQuoter upgrade (commit 523a0cc) correctly surfaces this error now — previously it would have returned garbage values silently.

## Fix Direction

Two options:

**Option A (Recommended)**: Handle `UnexpectedRevert` in the test — when the on-chain quote reverts with an `UnexpectedRevert` whose inner error is a `WrappedError` containing `Panic(0x11)`, skip that swap amount and continue (similar to the out-of-range skip). This is correct because the on-chain hook legitimately rejects swaps where MEV tax > output.

**Option B**: Reduce `priority_fee` to a realistic L2 value (e.g., 10,000 wei = 0.01 gwei). This avoids the underflow for all test amounts but reduces MEV tax coverage to trivially small tax values, making the test less useful.

A combination may be ideal: use a moderate priority fee AND handle the revert gracefully for edge cases.
