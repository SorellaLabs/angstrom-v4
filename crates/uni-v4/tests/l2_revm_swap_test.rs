use std::sync::Arc;

use alloy::{
    network::Ethereum,
    node_bindings::Anvil,
    primitives::{Address, I256, Uint, address},
    providers::{Provider, ProviderBuilder},
    sol
};
use op_alloy_network::Optimism;
use uni_v4::l2_structure::{L2AddressBook, pool_registry::L2PoolRegistry};
use uni_v4_structure::pool_registry::PoolRegistry;
use uni_v4_upkeeper::pool_manager_service_builder::PoolManagerServiceBuilder;

fn get_base_url() -> Option<String> {
    dotenv::dotenv().ok();
    std::env::var("BASE_URL").ok()
}

const POOL_MANAGER: Address = address!("0x498581ff718922c3f8e6a244956af099b2652b2b");
const ANGSTROM_L2_FACTORY: Address = address!("0x0000000000a5f21b113a18dd18f6fbeebd01201b");
const DEPLOY_BLOCK: u64 = 42966000;
const TARGET_BLOCK: u64 = 42977290;

const CBBTC: Address = address!("0xcbB7C0000aB88B473b1f5aFd9ef808440eed33Bf");

// Small amount that stays within a single tick (~0.001 cbBTC, 8 decimals)
const SMALL_AMOUNT: i64 = 100_000;
// Larger amount designed to cross tick boundaries (~0.1 cbBTC)
const LARGE_AMOUNT: i64 = 10_000_000;

type U160 = Uint<160, 3>;

// V4 sqrt price limits (uint160)
// MIN_SQRT_PRICE + 1
const MIN_SQRT_PRICE_LIMIT: U160 = U160::from_limbs([4295128740, 0, 0]);
// MAX_SQRT_PRICE - 1 = 1461446703485210103287273052203988822378723970341
const MAX_SQRT_PRICE_LIMIT: U160 =
    U160::from_limbs([6743328256752651557, 17280870778742802505, 4294805859]);

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

    #[sol(rpc, bytecode = "0x60a0604052348015600e575f80fd5b506040516107cc3803806107cc833981016040819052602b91603b565b6001600160a01b03166080526066565b5f60208284031215604a575f80fd5b81516001600160a01b0381168114605f575f80fd5b9392505050565b60805161073b6100915f395f818160480152818160d50152818161011601526101e2015261073b5ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c8063481c6a751461004357806391dd734614610087578063d34f8f9a146100a7575b5f80fd5b61006a7f000000000000000000000000000000000000000000000000000000000000000081565b6040516001600160a01b0390911681526020015b60405180910390f35b61009a610095366004610316565b6100c8565b60405161007e91906103b0565b6100ba6100b5366004610542565b6101de565b60405161007e929190610610565b6060336001600160a01b037f000000000000000000000000000000000000000000000000000000000000000016146100fe575f80fd5b5f808061010d85870187610542565b9250925092505f7f00000000000000000000000000000000000000000000000000000000000000006001600160a01b031663f3cd914c8585856040518463ffffffff1660e01b815260040161016493929190610624565b6020604051808303815f875af1158015610180573d5f803e3d5ffd5b505050506040513d601f19601f820116820180604052508101906101a491906106a3565b90506101b08160801d90565b6101ba82600f0b90565b604051633809ba3d60e11b81526004016101d5929190610610565b60405180910390fd5b5f807f00000000000000000000000000000000000000000000000000000000000000006001600160a01b03166348c8949186868660405160200161022493929190610624565b6040516020818303038152906040526040518263ffffffff1660e01b815260040161024f91906103b0565b5f604051808303815f875af192505050801561028c57506040513d5f823e601f3d908101601f1916820160405261028991908101906106ba565b60015b6102d3573d8080156102b9576040519150601f19603f3d011682016040523d82523d5f602084013e6102be565b606091505b5060248101519250604481015191505061030e565b5060405162461bcd60e51b815260206004820152600f60248201526e115e1c1958dd1959081c995d995c9d608a1b60448201526064016101d5565b935093915050565b5f8060208385031215610327575f80fd5b82356001600160401b0381111561033c575f80fd5b8301601f8101851361034c575f80fd5b80356001600160401b03811115610361575f80fd5b856020828401011115610372575f80fd5b6020919091019590945092505050565b5f81518084528060208401602086015e5f602082860101526020601f19601f83011685010191505092915050565b602081525f6103c26020830184610382565b9392505050565b634e487b7160e01b5f52604160045260245ffd5b60405160a081016001600160401b03811182821017156103ff576103ff6103c9565b60405290565b604051601f8201601f191681016001600160401b038111828210171561042d5761042d6103c9565b604052919050565b6001600160a01b0381168114610449575f80fd5b50565b803561045781610435565b919050565b5f6060828403121561046c575f80fd5b604051606081016001600160401b038111828210171561048e5761048e6103c9565b604052905080823580151581146104a3575f80fd5b81526020838101359082015260408301356104bd81610435565b6040919091015292915050565b5f6001600160401b038211156104e2576104e26103c9565b50601f01601f191660200190565b5f82601f8301126104ff575f80fd5b813561051261050d826104ca565b610405565b818152846020838601011115610526575f80fd5b816020850160208301375f918101602001919091529392505050565b5f805f838503610120811215610556575f80fd5b60a0811215610563575f80fd5b5061056c6103dd565b843561057781610435565b8152602085013561058781610435565b6020820152604085013562ffffff811681146105a1575f80fd5b60408201526060850135600281900b81146105ba575f80fd5b60608201526105cb6080860161044c565b608082015292506105df8560a0860161045c565b91506101008401356001600160401b038111156105fa575f80fd5b610606868287016104f0565b9150509250925092565b600f92830b8152910b602082015260400190565b83516001600160a01b03908116825260208086015182168184015260408087015162ffffff168185015260608088015160020b908501526080808801518416908501528551151560a08501529085015160c08401528401511660e08201526101206101008201525f61069a610120830184610382565b95945050505050565b5f602082840312156106b3575f80fd5b5051919050565b5f602082840312156106ca575f80fd5b81516001600160401b038111156106df575f80fd5b8201601f810184136106ef575f80fd5b80516106fd61050d826104ca565b818152856020838501011115610711575f80fd5b8160208401602083015e5f9181016020019190915294935050505056fea164736f6c634300081a000a")]
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

fn make_pool_key(registry: &L2PoolRegistry, pool_id: &alloy::primitives::B256) -> PoolKey {
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
async fn test_l2_swap_matches_onchain() {
    let Some(base_url) = get_base_url() else {
        println!("No BASE_URL set, skipping");
        return;
    };

    let provider = Arc::new(
        ProviderBuilder::<_, _, Optimism>::default()
            .with_recommended_fillers()
            .connect(&base_url)
            .await
            .unwrap()
    );

    let service = PoolManagerServiceBuilder::new_with_noop_stream(
        provider.clone(),
        L2AddressBook::new(ANGSTROM_L2_FACTORY),
        L2PoolRegistry::default(),
        POOL_MANAGER,
        DEPLOY_BLOCK
    )
    .with_initial_tick_range_size(6000)
    .with_auto_pool_creation(true)
    .with_current_block(TARGET_BLOCK)
    .build()
    .await
    .expect("Failed to create service");

    let pools = service.get_pools();

    let (pool_id, _) = pools
        .get_pools()
        .iter()
        .find(|entry| entry.value().token0 == Address::ZERO && entry.value().token1 == CBBTC)
        .map(|entry| (*entry.key(), entry.value().clone()))
        .expect("ETH/CBBTC pool not found");

    let pool_state = pools.get_pools().get(&pool_id).expect("pool disappeared");
    assert!(pool_state.current_liquidity() > 0, "Pool has no liquidity");

    println!(
        "Pool: tick={}, liquidity={}, tick_spacing={}",
        pool_state.current_tick(),
        pool_state.current_liquidity(),
        pool_state.tick_spacing()
    );

    let registry = service.get_registry();
    let pool_key = make_pool_key(&registry, &pool_id);

    // Anvil fork + deploy quoter
    let anvil = Anvil::new()
        .fork(&base_url)
        .fork_block_number(TARGET_BLOCK)
        .spawn();

    let anvil_provider =
        ProviderBuilder::<_, _, Ethereum>::default().connect_http(anvil.endpoint_url());
    let quoter = SwapQuoter::deploy(&anvil_provider, POOL_MANAGER)
        .await
        .expect("Failed to deploy SwapQuoter");

    println!("SwapQuoter deployed at {:?}", quoter.address());

    for (zero_for_one, dir_label) in [(false, "CBBTC->ETH"), (true, "ETH->CBBTC")] {
        for (amount_raw, size_label) in [(SMALL_AMOUNT, "small"), (LARGE_AMOUNT, "large")] {
            let label = format!("{dir_label} {size_label}");

            // Local: positive I256 = exact input
            let local_result = pool_state
                .swap_current_with_amount(I256::try_from(amount_raw).unwrap(), zero_for_one, false)
                .unwrap_or_else(|e| panic!("Local swap failed ({label}): {e}"));

            // On-chain: negative amountSpecified = exact input
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
                .call()
                .await
                .unwrap_or_else(|e| panic!("On-chain quote failed ({label}): {e}"));

            println!(
                "{label}: local t0={} t1={} | onchain a0={} a1={}",
                local_result.total_d_t0, local_result.total_d_t1, result.amount0, result.amount1
            );

            assert_deltas_match(
                local_result.total_d_t0,
                local_result.total_d_t1,
                result.amount0,
                result.amount1,
                zero_for_one,
                &label
            );
            println!("PASS: {label}");
        }
    }

    drop(anvil);
}

#[tokio::test]
async fn test_l2_swap_with_mev_tax_matches_onchain() {
    let Some(base_url) = get_base_url() else {
        println!("No BASE_URL set, skipping");
        return;
    };

    let provider = Arc::new(
        ProviderBuilder::<_, _, Optimism>::default()
            .with_recommended_fillers()
            .connect(&base_url)
            .await
            .unwrap()
    );

    let service = PoolManagerServiceBuilder::new_with_noop_stream(
        provider.clone(),
        L2AddressBook::new(ANGSTROM_L2_FACTORY),
        L2PoolRegistry::default(),
        POOL_MANAGER,
        DEPLOY_BLOCK
    )
    .with_initial_tick_range_size(6000)
    .with_auto_pool_creation(true)
    .with_current_block(TARGET_BLOCK)
    .build()
    .await
    .expect("Failed to create service");

    let pools = service.get_pools();

    let (pool_id, _) = pools
        .get_pools()
        .iter()
        .find(|entry| entry.value().token0 == Address::ZERO && entry.value().token1 == CBBTC)
        .map(|entry| (*entry.key(), entry.value().clone()))
        .expect("ETH/CBBTC pool not found");

    let pool_state = pools.get_pools().get(&pool_id).expect("pool disappeared");
    assert!(pool_state.current_liquidity() > 0, "Pool has no liquidity");

    let registry = service.get_registry();
    let pool_key = make_pool_key(&registry, &pool_id);

    // Anvil fork + deploy quoter
    let anvil = Anvil::new()
        .fork(&base_url)
        .fork_block_number(TARGET_BLOCK)
        .spawn();

    let anvil_provider =
        ProviderBuilder::<_, _, Ethereum>::default().connect_http(anvil.endpoint_url());
    let quoter = SwapQuoter::deploy(&anvil_provider, POOL_MANAGER)
        .await
        .expect("Failed to deploy SwapQuoter");

    // Get basefee from the forked block
    let block = anvil_provider
        .get_block_by_number(alloy::eips::BlockNumberOrTag::Latest)
        .await
        .expect("failed to get block")
        .expect("block not found");
    let basefee = block.header.base_fee_per_gas.expect("no basefee");

    let priority_fee: u128 = 1_000_000_000; // 1 gwei
    let gas_price = basefee as u128 + priority_fee;

    println!("basefee={basefee}, priority_fee={priority_fee}, gas_price={gas_price}");

    for (zero_for_one, dir_label) in [(false, "CBBTC->ETH"), (true, "ETH->CBBTC")] {
        for (amount_raw, size_label) in [(SMALL_AMOUNT, "small"), (LARGE_AMOUNT, "large")] {
            let label = format!("{dir_label} {size_label} mev_tax");

            // Local swap with MEV tax
            let local_result = pool_state
                .swap_current_with_amount_and_mev_tax(
                    I256::try_from(amount_raw).unwrap(),
                    zero_for_one,
                    false,
                    Some(priority_fee)
                )
                .unwrap_or_else(|e| panic!("Local swap failed ({label}): {e}"));

            // On-chain quote with elevated gas price
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
                .gas_price(gas_price)
                .call()
                .await
                .unwrap_or_else(|e| panic!("On-chain quote failed ({label}): {e}"));

            println!(
                "{label}: local t0={} t1={} | onchain a0={} a1={}",
                local_result.total_d_t0, local_result.total_d_t1, result.amount0, result.amount1
            );

            assert_deltas_match(
                local_result.total_d_t0,
                local_result.total_d_t1,
                result.amount0,
                result.amount1,
                zero_for_one,
                &label
            );
            println!("PASS: {label}");
        }
    }

    drop(anvil);
}
