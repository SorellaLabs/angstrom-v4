use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration
};

use alloy::{
    eips::BlockId,
    network::Ethereum,
    node_bindings::Anvil,
    primitives::{Address, I256, U256, Uint, address},
    providers::{Provider, ProviderBuilder, ext::AnvilApi},
    rpc::types::Block,
    sol
};
use futures::{Stream, future::BoxFuture};
use uni_v4::{
    L1AddressBook, PoolId,
    pool_providers::update_provider::{PoolUpdateProvider, StateStream},
    pool_registry::L1PoolRegistry
};
use uni_v4_structure::pool_registry::PoolRegistry;
use uni_v4_upkeeper::{
    pool_manager_service_builder::PoolManagerServiceBuilder, slot0::NoOpSlot0Stream
};

fn get_eth_url() -> Option<String> {
    dotenv::dotenv().ok();
    std::env::var("ETH_URL").ok()
}

/// Angstrom storage slot 3 packs `_lastBlockUpdated` (low 64 bits) with
/// `_configStore` address (bits 64+).  Setting the low 64 bits to the current
/// block number makes `_isUnlocked()` return true so the hook allows swaps.
const ANGSTROM_LAST_BLOCK_SLOT: U256 = U256::from_limbs([3, 0, 0, 0]);

// Mainnet addresses (chain_id=1)
const ANGSTROM: Address = address!("0x0000000AA8c2Fb9b232F78D2B286dC2aE53BfAD4");
const CONTROLLER_V1: Address = address!("0x16eD937987753a50f9Eb293eFffA753aC4313db0");
const POOL_MANAGER: Address = address!("0x000000000004444c5dc75cB358380D2e3dE08A90");
const DEPLOY_BLOCK: u64 = 22689729;

// 100 consecutive blocks starting from a recent mainnet block.
// UPDATE: set INITIAL_BLOCK to a recent block when running.
const INITIAL_BLOCK: u64 = 22750000;
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
    (1_000_000_000_000_000_000, "1 ETH")
];

/// Block stream that fetches a specific range of historical blocks
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

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if self.current_block > self.end_block {
                return Poll::Ready(None);
            }

            if let Some(mut future) = self.pending_future.take() {
                match future.as_mut().poll(cx) {
                    Poll::Ready(Some(block)) => {
                        self.current_block += 1;
                        return Poll::Ready(Some(block));
                    }
                    Poll::Ready(None) => {
                        self.current_block += 1;
                        continue;
                    }
                    Poll::Pending => {
                        self.pending_future = Some(future);
                        return Poll::Pending;
                    }
                }
            } else {
                let provider = self.provider.clone();
                let block_num = self.current_block;

                let future = Box::pin(async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    match provider.get_block(BlockId::Number(block_num.into())).await {
                        Ok(Some(block)) => Some(block),
                        _ => None
                    }
                });

                self.pending_future = Some(future);
            }
        }
    }
}

sol! {
    struct PoolKey {
        address currency0;
        address currency1;
        uint24 fee;
        int24 tickSpacing;
        address hooks;
    }

    struct SwapParams {
        bool zeroForOne;
        int256 amountSpecified;
        uint160 sqrtPriceLimitX96;
    }

    #[sol(rpc, bytecode = "0x60a0604052348015600e575f80fd5b50604051610813380380610813833981016040819052602b91603b565b6001600160a01b03166080526066565b5f60208284031215604a575f80fd5b81516001600160a01b0381168114605f575f80fd5b9392505050565b6080516107826100915f395f818160480152818160d50152818161011601526101e201526107825ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c8063481c6a751461004357806391dd734614610087578063d34f8f9a146100a7575b5f80fd5b61006a7f000000000000000000000000000000000000000000000000000000000000000081565b6040516001600160a01b0390911681526020015b60405180910390f35b61009a61009536600461035d565b6100c8565b60405161007e91906103f7565b6100ba6100b5366004610589565b6101de565b60405161007e929190610657565b6060336001600160a01b037f000000000000000000000000000000000000000000000000000000000000000016146100fe575f80fd5b5f808061010d85870187610589565b9250925092505f7f00000000000000000000000000000000000000000000000000000000000000006001600160a01b031663f3cd914c8585856040518463ffffffff1660e01b81526004016101649392919061066b565b6020604051808303815f875af1158015610180573d5f803e3d5ffd5b505050506040513d601f19601f820116820180604052508101906101a491906106ea565b90506101b08160801d90565b6101ba82600f0b90565b604051633809ba3d60e11b81526004016101d5929190610657565b60405180910390fd5b5f807f00000000000000000000000000000000000000000000000000000000000000006001600160a01b03166348c894918686866040516020016102249392919061066b565b6040516020818303038152906040526040518263ffffffff1660e01b815260040161024f91906103f7565b5f604051808303815f875af192505050801561028c57506040513d5f823e601f3d908101601f191682016040526102899190810190610701565b60015b61031a573d8080156102b9576040519150601f19603f3d011682016040523d82523d5f602084013e6102be565b606091505b5060208101516001600160e01b03198116633809ba3d60e11b1415806102e5575060448251105b15610305578160405163311ef19f60e21b81526004016101d591906103f7565b60248201519350604482015192505050610355565b5060405162461bcd60e51b815260206004820152600f60248201526e115e1c1958dd1959081c995d995c9d608a1b60448201526064016101d5565b935093915050565b5f806020838503121561036e575f80fd5b82356001600160401b03811115610383575f80fd5b8301601f81018513610393575f80fd5b80356001600160401b038111156103a8575f80fd5b8560208284010111156103b9575f80fd5b6020919091019590945092505050565b5f81518084528060208401602086015e5f602082860101526020601f19601f83011685010191505092915050565b602081525f61040960208301846103c9565b9392505050565b634e487b7160e01b5f52604160045260245ffd5b60405160a081016001600160401b038111828210171561044657610446610410565b60405290565b604051601f8201601f191681016001600160401b038111828210171561047457610474610410565b604052919050565b6001600160a01b0381168114610490575f80fd5b50565b803561049e8161047c565b919050565b5f606082840312156104b3575f80fd5b604051606081016001600160401b03811182821017156104d5576104d5610410565b604052905080823580151581146104ea575f80fd5b81526020838101359082015260408301356105048161047c565b6040919091015292915050565b5f6001600160401b0382111561052957610529610410565b50601f01601f191660200190565b5f82601f830112610546575f80fd5b813561055961055482610511565b61044c565b81815284602083860101111561056d575f80fd5b816020850160208301375f918101602001919091529392505050565b5f805f83850361012081121561059d575f80fd5b60a08112156105aa575f80fd5b506105b3610424565b84356105be8161047c565b815260208501356105ce8161047c565b6020820152604085013562ffffff811681146105e8575f80fd5b60408201526060850135600281900b8114610601575f80fd5b606082015261061260808601610493565b608082015292506106268560a086016104a3565b91506101008401356001600160401b03811115610641575f80fd5b61064d86828701610537565b9150509250925092565b600f92830b8152910b602082015260400190565b83516001600160a01b03908116825260208086015182168184015260408087015162ffffff168185015260608088015160020b908501526080808801518416908501528551151560a08501529085015160c08401528401511660e08201526101206101008201525f6106e16101208301846103c9565b95945050505050565b5f602082840312156106fa575f80fd5b5051919050565b5f60208284031215610711575f80fd5b81516001600160401b03811115610726575f80fd5b8201601f81018413610736575f80fd5b805161074461055482610511565b818152856020838501011115610758575f80fd5b8160208401602083015e5f9181016020019190915294935050505056fea164736f6c634300081a000a")]
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

fn assert_deltas_match(
    local_d_t0: u128,
    local_d_t1: u128,
    onchain_amount0: i128,
    onchain_amount1: i128,
    zero_for_one: bool,
    label: &str
) {
    if zero_for_one {
        assert!(onchain_amount0 <= 0, "{label}: expected negative amount0, got {onchain_amount0}");
        assert!(onchain_amount1 >= 0, "{label}: expected positive amount1, got {onchain_amount1}");
    } else {
        assert!(onchain_amount1 <= 0, "{label}: expected negative amount1, got {onchain_amount1}");
        assert!(onchain_amount0 >= 0, "{label}: expected positive amount0, got {onchain_amount0}");
    }
    assert_eq!(local_d_t0, onchain_amount0.unsigned_abs(), "{label}: token0 delta mismatch");
    assert_eq!(local_d_t1, onchain_amount1.unsigned_abs(), "{label}: token1 delta mismatch");
}

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
    let block_stream = HistoricalBlockStream::new(provider.clone(), INITIAL_BLOCK + 1, FINAL_BLOCK);

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

                // Local swap (no MEV tax for L1)
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

                // Unlock the Angstrom hook for the current anvil block so beforeSwap doesn't revert.
                // Read slot 3, preserve upper bits (_configStore), overwrite low 64 bits with block.number.
                let current_block = anvil_provider
                    .get_block_number()
                    .await
                    .expect("failed to get block number");
                let slot_val = anvil_provider
                    .get_storage_at(ANGSTROM, ANGSTROM_LAST_BLOCK_SLOT)
                    .await
                    .expect("failed to read slot");
                let mask = U256::from(u64::MAX);
                let new_val = (slot_val & !mask) | U256::from(current_block);
                anvil_provider
                    .anvil_set_storage_at(ANGSTROM, ANGSTROM_LAST_BLOCK_SLOT, new_val.into())
                    .await
                    .expect("failed to set storage");

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

                println!(
                    "{label}: local t0={} t1={} | onchain a0={} a1={}",
                    local_result.total_d_t0,
                    local_result.total_d_t1,
                    result.amount0,
                    result.amount1
                );

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
