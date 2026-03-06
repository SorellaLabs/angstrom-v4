---
stage: research
pipeline: bug
timestamp: 2026-03-06T00:00:00Z
arguments: "test_l1_swap_replay_matches_onchain token0 delta mismatch: local 1e15 vs onchain 1.16e38"
---

# Bug Research: L1 Swap Delta Mismatch

## Summary

The `test_l1_swap_replay_matches_onchain` test fails because the on-chain SwapQuoter returns garbage swap deltas (`a0=-115959654862451768547423990981983470892, a1=0`) instead of real values. The root cause is a **two-part failure**: (1) the Angstrom hook's `_isUnlocked()` check fails because `_lastBlockUpdated` is set to `current_block + 1` instead of the value `block.number` will have during `eth_call`, causing the hook to revert with `CannotSwapWhileLocked()`; (2) the SwapQuoter contract blindly extracts bytes from ANY revert data at fixed offsets without validating the error selector, so the V4 `WrappedError` (containing the hook address and beforeSwap selector) is misinterpreted as `(int128 amount0, int128 amount1)`.

## Reproduction

1. Set `ETH_URL` env var to an Ethereum mainnet RPC
2. Run: `cargo test --package uni-v4 --test l1_revm_swap_test -- test_l1_swap_replay_matches_onchain --exact --nocapture`
3. Test replays 100 blocks, then compares local swap simulation vs on-chain quote via Anvil fork
4. First ZFO swap on the USDC/WETH pool produces garbage on-chain deltas

## Root Cause

### Primary: Wrong `_lastBlockUpdated` value (l1_revm_swap_test.rs:311)

```rust
// Line 311 — the +1 is the bug
let new_val = (slot_val & !mask) | U256::from(current_block + 1);
```

The Angstrom hook's unlock check at `TopLevelAuth.sol:213-214`:
```solidity
function _isUnlocked() internal view returns (bool) {
    return _lastBlockUpdated == block.number;
}
```

The test sets `_lastBlockUpdated = get_block_number() + 1`. During `eth_call`:
- If Anvil uses `block.number = latest_block` (standard behavior): `_lastBlockUpdated = N+1 != N` → **LOCKED**
- If Anvil uses `block.number = latest_block + 1` (pending behavior): they match → **UNLOCKED**

Standard `eth_call` behavior uses the latest block's number, making `+1` incorrect. The hook remains locked.

### Secondary: SwapQuoter doesn't validate revert error selectors

When the hook is locked and empty hookData is passed (`vec![].into()`), the hook reverts with `CannotSwapWhileLocked()` (UnlockHook.sol:32). This propagates through:

1. `hook.beforeSwap()` → reverts `CannotSwapWhileLocked()`
2. V4 `Hooks.callHook()` → wraps in `WrappedError(address, bytes4, bytes, bytes)` via `CustomRevert.bubbleUpAndRevertWith()` (Hooks.sol:136)
3. `manager.swap()` → reverts with WrappedError
4. `manager.unlock()` → reverts with WrappedError
5. `quoter.quote()` → catches revert, extracts bytes at offsets 0x24 and 0x44

The WrappedError layout:
```
[0x00:0x04] WrappedError selector
[0x04:0x24] revertingContract = Angstrom address (0x0000000AA8c2Fb9b...)
[0x24:0x44] revertingFunctionSelector = beforeSwap selector
[0x44:0x64] offset to revert reason = 0x80
...
```

The quoter reads:
- `amount0` ← bytes at 0x24 = **Angstrom address** → garbage int128 (`-115959654862451768547423990981983470892`)
- `amount1` ← bytes at 0x44 = **0x80** (offset constant) → but after int128 cast = `0`

This matches the test output exactly: `a0=-115959654862451768547423990981983470892 a1=0`.

## Code Analysis

### The unlock mechanism (l1_revm_swap_test.rs:300-315)

```rust
let current_block = anvil_provider
    .get_block_number().await.expect("failed to get block number");
let slot_val = anvil_provider
    .get_storage_at(ANGSTROM, ANGSTROM_LAST_BLOCK_SLOT).await
    .expect("failed to read slot");
let mask = U256::from(u64::MAX);
let new_val = (slot_val & !mask) | U256::from(current_block + 1);  // BUG: +1
anvil_provider
    .anvil_set_storage_at(ANGSTROM, ANGSTROM_LAST_BLOCK_SLOT, new_val.into()).await
    .expect("failed to set storage");
```

- Storage slot 3 is correct: `_lastBlockUpdated` (uint64, low 64 bits) + `_configStore` (address, bits 64-223) pack into one 32-byte slot
- `anvil_set_storage_at` does NOT mine a new block — it modifies state in-place
- `get_block_number()` returns the latest mined block number

### The quoter contract (embedded bytecode in sol! macro, l1_revm_swap_test.rs:135-144)

The `quote()` function catches any revert from `manager.unlock()` and extracts two int128 values from fixed offsets. There is **no validation** of the error selector — if the inner call reverts for any reason (hook locked, gas OOG, invalid pool), the quoter silently returns garbage data instead of propagating the error.

### The local swap path (pool_swap.rs:31-248)

The local simulation correctly handles L1 fees:
- `l2_fees()` returns `false` for L1 → skips BeforeSwapDelta logic (line 58)
- Protocol fee applied AFTER the AMM swap (lines 194-233)
- This matches the Angstrom L1 hook behavior: `beforeSwap` returns `BeforeSwapDelta.wrap(0)` and `afterSwap` takes the protocol fee

The local path is correct — the mismatch is entirely in the on-chain quote.

## External Knowledge

- **Uniswap V4 WrappedError format**: V4 uses ERC-7751 wrapped errors via `CustomRevert.bubbleUpAndRevertWith()`. When a hook reverts, the error is wrapped with the hook address and calling function selector, creating a structured error that callers can inspect.
- **Anvil eth_call block.number**: Standard behavior for `eth_call` at "latest" uses the latest mined block number. Some clients may default to "pending" (latest + 1). The `+1` in the test was likely added assuming pending block execution.
- **anvil_setStorageAt**: This is a cheat code that modifies state directly without mining a block or incrementing block.number.

## Impact Assessment

- **Direct**: The L1 swap replay integration test is broken — it cannot validate local sim accuracy against on-chain
- **No production impact**: This is test-only code; the local swap simulation (`pool_swap.rs`) is correct
- **False confidence risk**: Without this test passing, regressions in the local swap simulation could go undetected

## Related Areas

1. **L2 swap test** (`l2_revm_swap_test.rs`): May have a similar quoter issue if it uses the same SwapQuoter contract. Should verify it handles hook unlock correctly.
2. **SwapQuoter robustness**: The quoter should validate the revert error selector matches its expected custom error, or at minimum propagate unexpected errors rather than returning garbage. This violates the "never silently default on failure paths" principle.

## Relevant History

- `e39ba17` — "fix: unlock Angstrom hook before each on-chain quote in L1 swap test" — This commit added the unlock mechanism (lines 300-315). The `+1` offset was introduced here.
- `7436bd0` — "test: add L1 swap replay integration test comparing local sim vs on-chain over 100 blocks" — Original test creation, without unlock logic.

## Fix Recommendation (for plan stage)

1. **Primary fix**: Change `current_block + 1` to `current_block` on line 311. If Anvil's `eth_call` actually uses `latest + 1`, change it to match that. The correct value is whatever `block.number` evaluates to during the `eth_call`.
2. **Secondary fix**: Improve the SwapQuoter contract to validate the revert selector, or have the Rust test check for reasonable values before asserting (e.g., verify amount0 sign matches direction, verify magnitudes are plausible).
3. **Verification**: Add a debug assertion or log in the test to confirm the hook is actually unlocked before quoting (e.g., read `_lastBlockUpdated` after setting it and compare with the block number the quote will run against).
