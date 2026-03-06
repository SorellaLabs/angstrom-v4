---
stage: plan
pipeline: feature
timestamp: 2026-03-06T00:00:00Z
arguments: Add L1 integration swap tests like L2, replaying 100 consecutive blocks with updates, comparing local vs on-chain swaps for all discovered pools.
---

# Plan: L1 Integration Swap Replay Test

## Goal

Create an L1 (Ethereum mainnet) integration swap test that mirrors the L2 `l2_revm_swap_test.rs` pattern but replays over 100 consecutive blocks. The test:
1. Loads pool state at a starting block via auto-discovery
2. Streams 100 consecutive block updates (applying swaps, liquidity changes, fee updates)
3. After all updates, verifies swap accuracy on ALL pools with liquidity by comparing local simulation against Anvil-forked on-chain quotes

This tests that our incremental update mechanism maintains swap accuracy over extended block ranges on mainnet, where activity is significantly higher than L2.

## Approach

Create a single new test file `crates/uni-v4/tests/l1_revm_swap_test.rs` that combines:
- **Block streaming with updates** from `l1_integration_test.rs` (HistoricalBlockStream + PoolUpdateProvider + StateStream)
- **Swap comparison via Anvil fork** from `l2_revm_swap_test.rs` (SwapQuoter deployment, local vs on-chain delta assertion)

**Key L1 differences from L2:**
- Uses `Ethereum` network type (not `Optimism`)
- `L1AddressBook` with `controller_v1` + `angstrom` (not `L2AddressBook`)
- `L1PoolRegistry` with Angstrom pool ID mapping (not `L2PoolRegistry`)
- No MEV tax — `swap_current_with_amount(amount, direction, false)` only
- L1 protocol fee applied after swap (not via BeforeSwapDelta)
- `ETH_URL` env var (not `BASE_URL`)

**Mainnet addresses** (from `angstrom/crates/types/src/primitive/contract/mod.rs`, chain_id=1):
- Angstrom: `0x0000000AA8c2Fb9b232F78D2B286dC2aE53BfAD4`
- Controller V1: `0x16eD937987753a50f9Eb293eFffA753aC4313db0`
- Pool Manager: `0x000000000004444c5dc75cB358380D2e3dE08A90`
- Deploy Block: `22689729`

## Options Considered

1. **Fresh service at each of 100 blocks** — Load fresh state 100 times, compare swaps each time. Simple but expensive (100 full initializations) and doesn't test incremental updates.
2. **Stream 100 blocks, verify at end** (chosen) — One service, stream updates for 100 blocks, verify swaps at the final block. Tests both the update pipeline AND swap accuracy. Efficient and meaningful.
3. **Stream all blocks, verify at checkpoints** — Process 100 blocks but pause and verify at each. Complex (requires custom block-by-block control flow) for marginal additional value over option 2.

## Implementation Steps

### Step 1: Create the test file with constants and imports

**File:** `crates/uni-v4/tests/l1_revm_swap_test.rs` (new)

```rust
use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration
};

use alloy::{
    eips::BlockId,
    network::Ethereum,
    node_bindings::Anvil,
    primitives::{Address, I256, Uint, address},
    providers::{Provider, ProviderBuilder},
    rpc::types::Block,
    sol
};
use futures::{Stream, future::BoxFuture};
use uni_v4::{
    L1AddressBook, PoolId,
    pool_providers::update_provider::{PoolUpdateProvider, StateStream},
    pool_registry::L1PoolRegistry
};
use uni_v4_upkeeper::{
    pool_manager_service_builder::PoolManagerServiceBuilder,
    slot0::NoOpSlot0Stream
};

fn get_eth_url() -> Option<String> {
    dotenv::dotenv().ok();
    std::env::var("ETH_URL").ok()
}

// Mainnet addresses (chain_id=1)
const ANGSTROM: Address = address!("0x0000000AA8c2Fb9b232F78D2B286dC2aE53BfAD4");
const CONTROLLER_V1: Address = address!("0x16eD937987753a50f9Eb293eFffA753aC4313db0");
const POOL_MANAGER: Address = address!("0x000000000004444c5dc75cB358380D2e3dE08A90");
const DEPLOY_BLOCK: u64 = 22689729;

// 100 consecutive blocks starting from a recent mainnet block.
// UPDATE: set INITIAL_BLOCK to a recent block when running.
const INITIAL_BLOCK: u64 = 22750000; // <-- update to recent block
const NUM_BLOCKS: u64 = 100;
const FINAL_BLOCK: u64 = INITIAL_BLOCK + NUM_BLOCKS;

type U160 = Uint<160, 3>;
const MIN_SQRT_PRICE_LIMIT: U160 = U160::from_limbs([4295128740, 0, 0]);
const MAX_SQRT_PRICE_LIMIT: U160 =
    U160::from_limbs([6743328256752651557, 17280870778742802505, 4294805859]);

// Swap amounts in ascending order (ETH in wei)
const SWAP_AMOUNTS: &[(i64, &str)] = &[
    (1_000_000_000_000_000, "0.001 ETH"),
    (10_000_000_000_000_000, "0.01 ETH"),
    (100_000_000_000_000_000, "0.1 ETH"),
    (500_000_000_000_000_000, "0.5 ETH"),
    (1_000_000_000_000_000_000, "1 ETH"),
];
```

**Verification:** File compiles with `cargo check -p uni-v4`.

**Dependencies:** None — all imports already available in uni-v4's Cargo.toml.

### Step 2: Add HistoricalBlockStream (reuse from l1_integration_test.rs)

Same `HistoricalBlockStream` struct from `l1_integration_test.rs`. This is a `Stream<Item = Block>` that fetches blocks sequentially from the RPC.

```rust
pub struct HistoricalBlockStream<P: Provider> {
    provider:       Arc<P>,
    end_block:      u64,
    current_block:  u64,
    pending_future: Option<BoxFuture<'static, Option<Block>>>
}

impl<P: Provider> HistoricalBlockStream<P> {
    pub fn new(provider: Arc<P>, start_block: u64, end_block: u64) -> Self {
        Self { provider, end_block, current_block: start_block, pending_future: None }
    }
}

impl<P: Provider + 'static> Stream for HistoricalBlockStream<P> {
    type Item = Block;
    // ... same poll_next as l1_integration_test.rs
}
```

**Verification:** Struct compiles and is used by the test.

### Step 3: Add SwapQuoter contract and assertion helpers

Reuse the same SwapQuoter sol! macro and `assert_deltas_match` from `l2_revm_swap_test.rs`:

```rust
sol! {
    struct PoolKey { ... }
    struct SwapParams { ... }

    #[sol(rpc, bytecode = "0x60a06040...")]
    contract SwapQuoter {
        constructor(address _manager);
        function quote(
            PoolKey key,
            SwapParams params,
            bytes hookData
        ) external returns (int128 amount0, int128 amount1);
    }
}

fn assert_deltas_match(
    local_d_t0: u128, local_d_t1: u128,
    onchain_amount0: i128, onchain_amount1: i128,
    zero_for_one: bool, label: &str
) { ... }
```

Add an L1-specific helper to build PoolKey from the L1PoolRegistry:

```rust
fn make_pool_key(registry: &L1PoolRegistry, pool_id: &PoolId) -> PoolKey {
    let k = registry.get(pool_id).expect("pool not found in registry");
    PoolKey {
        currency0:   k.currency0,
        currency1:   k.currency1,
        fee:         k.fee.to::<u32>().try_into().unwrap(),
        tickSpacing: (k.tickSpacing.as_i32()).try_into().unwrap(),
        hooks:       k.hooks
    }
}
```

**Verification:** Compiles, `make_pool_key` returns correct PoolKey struct.

### Step 4: Implement the main test function

```rust
#[tokio::test]
async fn test_l1_swap_replay_matches_onchain() {
    let Some(eth_url) = get_eth_url() else {
        println!("No ETH_URL set, skipping");
        return;
    };

    let provider = Arc::new(
        ProviderBuilder::<_, _, Ethereum>::default()
            .with_recommended_fillers()
            .connect(&eth_url)
            .await
            .unwrap()
    );

    let address_book = L1AddressBook::new(CONTROLLER_V1, ANGSTROM);
    let pool_registry = L1PoolRegistry::new(ANGSTROM);

    // Step A: Create service with block stream for 100 blocks of updates
    println!("Creating block stream from {} to {}", INITIAL_BLOCK + 1, FINAL_BLOCK);
    let block_stream = HistoricalBlockStream::new(
        provider.clone(), INITIAL_BLOCK + 1, FINAL_BLOCK
    );

    let update_provider = PoolUpdateProvider::new_at_block(
        provider.clone(),
        POOL_MANAGER,
        address_book,
        pool_registry.clone(),
        INITIAL_BLOCK
    );

    let state_stream = StateStream::new(update_provider, block_stream);

    let mut service = PoolManagerServiceBuilder::<_, _, _, NoOpSlot0Stream>::new(
        provider.clone(),
        address_book,
        pool_registry.clone(),
        POOL_MANAGER,
        DEPLOY_BLOCK,
        state_stream
    )
    .with_initial_tick_range_size(6000)
    .with_auto_pool_creation(true)
    .with_current_block(INITIAL_BLOCK)
    .build()
    .await
    .expect("Failed to create service");

    // Step B: Process all 100 blocks of updates
    println!("Processing {NUM_BLOCKS} blocks of updates...");
    (&mut service).await;
    println!("All blocks processed");

    // Step C: Get updated pool states and registry
    let pools = service.get_pools();
    let registry = service.get_registry();
    let pool_count = pools.get_pools().len();
    println!("Found {pool_count} pools after replay");

    // Collect pools with liquidity
    let active_pools: Vec<(PoolId, _)> = pools
        .get_pools()
        .iter()
        .filter(|entry| entry.value().current_liquidity() > 0)
        .map(|entry| (*entry.key(), entry.value().clone()))
        .collect();

    println!("{} pools have liquidity", active_pools.len());
    assert!(!active_pools.is_empty(), "No pools with liquidity found after replay");

    // Step D: Fork Anvil at FINAL_BLOCK and deploy SwapQuoter
    let anvil = Anvil::new()
        .fork(&eth_url)
        .fork_block_number(FINAL_BLOCK)
        .spawn();

    let anvil_provider =
        ProviderBuilder::<_, _, Ethereum>::default().connect_http(anvil.endpoint_url());
    let quoter = SwapQuoter::deploy(&anvil_provider, POOL_MANAGER)
        .await
        .expect("Failed to deploy SwapQuoter");

    let block = anvil_provider
        .get_block_by_number(alloy::eips::BlockNumberOrTag::Latest)
        .await
        .expect("failed to get block")
        .expect("block not found");
    let basefee = block.header.base_fee_per_gas.expect("no basefee") as u128;

    // Step E: Compare swaps on all pools with liquidity
    let mut total_pass = 0u32;
    let mut total_skip = 0u32;

    for (pool_id, pool_state) in &active_pools {
        let pool_key = make_pool_key(&registry, pool_id);
        println!(
            "\nPool {:?}: tick={}, liq={}, t0={:?}, t1={:?}",
            pool_id,
            pool_state.current_tick(),
            pool_state.current_liquidity(),
            pool_state.token0,
            pool_state.token1
        );

        for (zero_for_one, dir_label) in [(true, "ZFO"), (false, "OFZ")] {
            for &(amount_raw, size_label) in SWAP_AMOUNTS {
                let label = format!("{dir_label} {size_label} pool={pool_id:?}");

                // Local swap (unlocked mode, no MEV tax for L1)
                let local_result = pool_state.swap_current_with_amount(
                    I256::try_from(amount_raw).unwrap(),
                    zero_for_one,
                    false
                );

                let local_result = match local_result {
                    Err(e) if e.to_string().contains("out of initialized tick ranges") => {
                        total_skip += 1;
                        continue;
                    }
                    Err(e) => panic!("Local swap failed ({label}): {e}"),
                    Ok(r) => r
                };

                // On-chain quote
                let sqrt_price_limit =
                    if zero_for_one { MIN_SQRT_PRICE_LIMIT } else { MAX_SQRT_PRICE_LIMIT };

                let result = quoter
                    .quote(
                        pool_key.clone(),
                        SwapParams {
                            zeroForOne:        zero_for_one,
                            amountSpecified:   I256::try_from(-amount_raw).unwrap(),
                            sqrtPriceLimitX96: sqrt_price_limit
                        },
                        vec![].into()
                    )
                    .gas_price(basefee)
                    .call()
                    .await
                    .unwrap_or_else(|e| panic!("On-chain quote failed ({label}): {e}"));

                assert_deltas_match(
                    local_result.total_d_t0,
                    local_result.total_d_t1,
                    result.amount0,
                    result.amount1,
                    zero_for_one,
                    &label
                );
                total_pass += 1;
            }
        }
    }

    println!("\n=== Results: {total_pass} passed, {total_skip} skipped (out-of-range) ===");
    drop(anvil);
}
```

**Verification:** Run with `cargo test -p uni-v4 --test l1_revm_swap_test -- --nocapture` with `ETH_URL` set. All assertions should pass.

### Step 5: Verify compilation and structure

**Verification criteria:**
- `cargo check -p uni-v4` succeeds
- `cargo +nightly fmt` passes
- `cargo clippy -p uni-v4` passes
- Test runs and passes with `ETH_URL` pointing to an Ethereum mainnet archive node

## Testing Plan

| Test | What it verifies |
|------|-----------------|
| `test_l1_swap_replay_matches_onchain` | After 100 blocks of incremental updates, local swap simulation matches Anvil-forked on-chain quotes for ALL pools in both directions at multiple amounts |

**Edge cases covered:**
- Pools with no liquidity (filtered out, not tested)
- Out-of-range swaps (skipped with count, not failures)
- Multiple swap sizes to test different tick-crossing scenarios
- Both swap directions (zero-for-one and one-for-zero)
- All auto-discovered pools (not just a specific pair)

**Manual verification:**
- Confirm `INITIAL_BLOCK` is recent enough that the RPC can serve it
- Confirm the Anvil fork at `FINAL_BLOCK` succeeds
- Check that at least some pools are discovered and have liquidity

## Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| RPC rate limits during 100-block stream | HistoricalBlockStream has 3s delay between blocks; increase if needed |
| INITIAL_BLOCK becomes stale | Comment in code instructs user to update; could add dynamic fetch later |
| Anvil fork fails for FINAL_BLOCK | The RPC must be an archive node; document this requirement |
| SwapQuoter bytecode incompatible with L1 hooks | Same bytecode works — it calls pool manager swap which triggers hooks automatically |
| L1 protocol fee calculation differs from on-chain | Our pool_swap.rs L1 fee path is already implemented; this test validates it |
| No pools discovered at DEPLOY_BLOCK | Angstrom mainnet is live with WETH/USDC and WETH/USDT; auto-discovery will find them |

## Follow-up Work

- Extract `HistoricalBlockStream` into a shared test utility (currently duplicated between `l1_integration_test.rs` and this new test)
- Add bundle-mode swap comparison (testing `is_bundle=true` path)
- Add dynamic INITIAL_BLOCK fetching (query latest block, subtract 100)
- Consider parallelizing pool swap comparisons for faster test execution
