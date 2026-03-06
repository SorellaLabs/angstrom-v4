---
stage: plan
pipeline: bug
timestamp: 2026-03-06T18:30:00Z
arguments: "look at pipeline. lets also upgrade tests contracts here"
---

# Plan: Upgrade SwapQuoter Test Contract

## Issue Summary

The `SwapQuoter.sol` test contract blindly decodes revert data at fixed memory offsets without validating the revert selector. When a swap reverts with a non-`QuoteResult` error (e.g., `HookDeltaExceedsSwapAmount`, `CannotSwapWhileLocked`, or any `WrappedError`), the quoter reads garbage memory and returns nonsensical values instead of failing loudly.

This violates the **"Never silently default on failure paths"** principle and has caused confusion across multiple pipelines:
- `bug-mev-tax-swap-mismatch`: MEV tax overwhelms small swaps → hook reverts → garbage values
- `l1-swap-delta-mismatch`: Hook not unlocked → hook reverts → garbage values

## Root Cause

`contracts/src/test/SwapQuoter.sol:26-32` — The `quote()` catch clause assumes every revert is a `QuoteResult(int128, int128)`:

```solidity
catch (bytes memory reason) {
    assembly {
        amount0 := mload(add(reason, 36))  // reads garbage if reason < 68 bytes
        amount1 := mload(add(reason, 68))  // reads garbage if reason < 100 bytes
    }
}
```

If the revert is a 4-byte selector-only error like `HookDeltaExceedsSwapAmount`, the `reason` data is only 4 bytes. Reading at offsets 36 and 68 returns uninitialized memory.

## Proposed Fix

### File 1: `contracts/src/test/SwapQuoter.sol`

Add selector validation before decoding. If the revert isn't `QuoteResult`, bubble up the original revert so the caller gets a clear error:

```solidity
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.24;

import {IPoolManager} from "v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "v4-core/src/types/PoolKey.sol";
import {SwapParams} from "v4-core/src/types/PoolOperation.sol";
import {BalanceDelta} from "v4-core/src/types/BalanceDelta.sol";
import {IUnlockCallback} from "v4-core/src/interfaces/callback/IUnlockCallback.sol";

contract SwapQuoter is IUnlockCallback {
    IPoolManager public immutable manager;

    error QuoteResult(int128 amount0, int128 amount1);
    error UnexpectedRevert(bytes reason);

    constructor(IPoolManager _manager) {
        manager = _manager;
    }

    function quote(
        PoolKey memory key,
        SwapParams memory params,
        bytes memory hookData
    ) external returns (int128 amount0, int128 amount1) {
        try manager.unlock(abi.encode(key, params, hookData)) {
            revert("Expected revert");
        } catch (bytes memory reason) {
            // Verify the revert is our QuoteResult error before decoding
            bytes4 selector;
            assembly {
                selector := mload(add(reason, 32))
            }
            if (selector != QuoteResult.selector || reason.length < 68) {
                revert UnexpectedRevert(reason);
            }
            assembly {
                amount0 := mload(add(reason, 36))
                amount1 := mload(add(reason, 68))
            }
        }
    }

    function unlockCallback(bytes calldata data) external returns (bytes memory) {
        require(msg.sender == address(manager));
        (PoolKey memory key, SwapParams memory params, bytes memory hookData) =
            abi.decode(data, (PoolKey, SwapParams, bytes));
        BalanceDelta delta = manager.swap(key, params, hookData);
        revert QuoteResult(delta.amount0(), delta.amount1());
    }
}
```

**Key changes:**
1. Added `error UnexpectedRevert(bytes reason)` — surfaces the original revert data instead of returning garbage
2. Extract 4-byte selector from position 32 (first 32 bytes of `reason` are the length prefix in memory)
3. Check `selector == QuoteResult.selector` AND `reason.length >= 68` (4-byte selector + 32-byte amount0 + 32-byte amount1)
4. If validation fails, revert with `UnexpectedRevert(reason)` — the caller sees the real error

### File 2: `crates/uni-v4/tests/l1_revm_swap_test.rs` (line 135)

Update the embedded bytecode in the `sol!` macro to use the newly compiled SwapQuoter. The bytecode string on line 135 must be replaced with the output of `forge build`, extracted from `contracts/out/SwapQuoter.sol/SwapQuoter.json` → `.bytecode.object`.

Also add the `UnexpectedRevert` error to the sol! ABI so Rust tests get a proper error type:

```rust
sol! {
    // ... PoolKey, SwapParams unchanged ...

    #[sol(rpc, bytecode = "<NEW_BYTECODE_HERE>")]
    contract SwapQuoter {
        error UnexpectedRevert(bytes reason);

        constructor(address _manager);

        function quote(
            PoolKey key,
            SwapParams params,
            bytes hookData
        ) external returns (int128 amount0, int128 amount1);
    }
}
```

### File 3: `crates/uni-v4/tests/l2_revm_swap_test.rs` (line 62)

Same bytecode and ABI update as File 2 — the L2 test has an identical `sol!` block.

## Execution Steps

1. **Edit `contracts/src/test/SwapQuoter.sol`** — apply the selector validation fix above
2. **Compile** — run `forge build` in the `contracts/` directory (requires foundry toolchain)
3. **Extract bytecode** — read `contracts/out/SwapQuoter.sol/SwapQuoter.json` → `.bytecode.object`
4. **Update `l1_revm_swap_test.rs`** — replace bytecode string on line 135, add `error UnexpectedRevert`
5. **Update `l2_revm_swap_test.rs`** — replace bytecode string on line 62, add `error UnexpectedRevert`
6. **Verify** — `cargo check --package uni-v4 --test l1_revm_swap_test --test l2_revm_swap_test`

## Why This Approach

- **Minimal change** — preserves the existing `(int128, int128)` return interface that all tests already use
- **Principle-compliant** — revert with `UnexpectedRevert` rather than silently returning garbage (never silently default on failure paths)
- **Uses existing imports** — no new library dependencies; just a selector comparison + length check
- **Matches v4-periphery pattern** — the official `QuoterRevert.sol` does the same thing (check selector, revert if unexpected) but for a different error type (`QuoteSwap`)

## Caveats

1. **forge required** — the bytecode must be recompiled. If `forge` is not available in the dev environment, the bytecode can be extracted from `contracts/out/` if a previous build exists, but ideally a fresh build should be done.
2. **V4 WrappedError** — when V4 wraps a hook revert in `WrappedError(address, bytes4, bytes, bytes)`, the outer selector will be `WrappedError.selector`, not `QuoteResult.selector`. The fix correctly catches this case (selector mismatch → `UnexpectedRevert`), which surfaces the full wrapped error to the Rust test.

## No Open Questions

No open questions — the plan is complete and ready for execution.
