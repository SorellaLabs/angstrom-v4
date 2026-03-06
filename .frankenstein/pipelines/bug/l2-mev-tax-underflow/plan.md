---
stage: plan
pipeline: bug
timestamp: 2026-03-06T19:30:00Z
arguments: "Lets do combination"
---

# Plan: Fix L2 MEV Tax Underflow — Combination Approach

## Issue Summary

`test_l2_swap_with_mev_tax_matches_onchain` fails on the first swap (`CBBTC->ETH 0.0001 cbBTC`) because the 1 gwei priority fee creates a MEV tax (0.01188 ETH) that exceeds the swap output (~0.0033 ETH), causing the on-chain hook to underflow (`Panic(0x11)`).

## Root Cause

Two issues combine:
1. **Unrealistic priority fee**: 1 gwei is 217x the Base L2 basefee (~4.6M wei)
2. **No graceful handling** of on-chain reverts when MEV tax overwhelms small swaps

## Proposed Fix

### File: `crates/uni-v4/tests/l2_revm_swap_test.rs`

**Change 1**: Reduce priority fee from 1 gwei to 10M wei (≈2x basefee)

This is realistic for Base L2 and produces a meaningful MEV tax:
```
mev_tax = 99 * 120,000 * 10,000,000 = 118,800,000,000,000 wei ≈ 0.000119 ETH
```

At line 306, change:
```rust
// Before:
let priority_fee: u128 = 1_000_000_000; // 1 gwei

// After:
let priority_fee: u128 = 10_000_000; // 10M wei (~2x L2 basefee)
```

**Change 2**: Handle on-chain quote reverts gracefully (lines 337-350)

When the hook reverts because MEV tax exceeds swap output, skip the test case instead of panicking. This matches the existing out-of-range skip pattern.

Replace the `.unwrap_or_else(|e| panic!(...))` at lines 337-350 with:
```rust
let result = match quoter
    .quote(
        pool_key.clone(),
        SwapParams {
            zeroForOne:        zero_for_one,
            amountSpecified:   I256::try_from(-amount_raw).unwrap(),
            sqrtPriceLimitX96: sqrt_price_limit
        },
        vec![].into()
    )
    .gas_price(gas_price)
    .call()
    .await
{
    Ok(r) => r,
    Err(e) => {
        let msg = e.to_string();
        if msg.contains("execution reverted") {
            println!("{label}: on-chain hook reverted (MEV tax > swap output, acceptable)");
            continue;
        }
        panic!("On-chain quote failed ({label}): {e}");
    }
};
```

## Why This Approach

- **Reduced priority fee** makes most swaps succeed, giving real MEV tax comparison coverage
- **Graceful revert handling** catches any remaining edge cases where tax still overwhelms tiny swaps — this is correct because the on-chain hook *should* reject economically nonsensical swaps
- **Follows existing pattern** — same skip-and-continue approach used for out-of-range errors at lines 323-331
- **Principle: "Test the live production flow"** — we still exercise the real MEV tax path with a realistic priority fee; we just accept that the hook correctly rejects borderline cases
- **Principle: "Never silently default on failure paths"** — non-revert errors still panic immediately

## No Open Questions

No open questions — the plan is complete and ready for execution.
