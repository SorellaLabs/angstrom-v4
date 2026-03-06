---
stage: execute
pipeline: feature
timestamp: 2026-03-06T00:00:00Z
arguments: Build the L1 integration swap replay test from the plan
---

# Execute: L1 Integration Swap Replay Test

## Steps Completed

- [x] Step 1: Create test file with constants and imports
- [x] Step 2: Add HistoricalBlockStream (copied from l1_integration_test.rs)
- [x] Step 3: Add SwapQuoter contract, assert_deltas_match, and L1-specific make_pool_key
- [x] Step 4: Implement main test function (test_l1_swap_replay_matches_onchain)
- [x] Step 5: Verify compilation and formatting

## Files Modified

- `crates/uni-v4/tests/l1_revm_swap_test.rs` (new)

## Changes Summary

Created a new L1 integration swap replay test that:
1. Loads pool state at INITIAL_BLOCK via auto-discovery
2. Streams 100 consecutive blocks of updates (swaps, liquidity changes, fee updates) using HistoricalBlockStream + PoolUpdateProvider + StateStream
3. After all updates, verifies swap accuracy on ALL pools with liquidity by comparing local simulation against Anvil-forked on-chain quotes
4. Tests 5 swap amounts (0.001 to 1 ETH) in both directions (ZFO and OFZ) for every active pool

Key L1 differences from L2 test:
- Uses `Ethereum` network type (not `Optimism`)
- `L1AddressBook` with `controller_v1` + `angstrom`
- `L1PoolRegistry` with Angstrom pool ID mapping
- No MEV tax — `swap_current_with_amount(amount, direction, false)` only
- `ETH_URL` env var (not `BASE_URL`)
- Mainnet addresses (chain_id=1)

## Verification Results

- `cargo check -p uni-v4 --test l1_revm_swap_test` — PASS
- `cargo +nightly fmt -- --check` — PASS
- `cargo clippy -p uni-v4 --test l1_revm_swap_test` — PASS
- Full test run requires `ETH_URL` pointing to an Ethereum mainnet archive node

## Deviations

None — implementation follows the plan exactly.

## Issues Discovered

None.

## Follow-up

- Extract `HistoricalBlockStream` into a shared test utility (currently duplicated between `l1_integration_test.rs` and this new test)
- Run the test with `ETH_URL` set to verify swap accuracy against mainnet
- Update `INITIAL_BLOCK` to a recent block before running
