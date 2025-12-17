use std::{collections::HashSet, sync::Arc, time::Duration};

use alloy_primitives::{address, I256, U160};
use alloy_provider::ProviderBuilder;
use alloy_network::Ethereum;
use jsonrpsee::ws_client::WsClientBuilder;
use uniswap_v3_math::sqrt_price::SqrtPriceX96;
use uni_v4_structure::{PoolId, pool_registry::l1::L1PoolRegistry, L1AddressBook};
use uni_v4_upkeeper::{
    pool_manager_service_builder::{PoolManagerServiceBuilder, NoOpEventStream},
    slot0::{NoOpSlot0Stream, Slot0Client}
};

/// Example demonstrating PoolManagerServiceBuilder with slot0 stream for
/// real-time updates
///
/// This example shows how to:
/// 1. Connect to an Angstrom WebSocket RPC for real-time slot0 updates
/// 2. Set up event stream for block-based pool state changes
/// 3. Use PoolManagerServiceBuilder to create a service with both streams
/// 4. Swapping with this pool.
#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Setup HTTP provider for general blockchain interaction
    let rpc_url = std::env::var("RPC_URL").expect("no rpc url set, must be ws");

    let provider = Arc::new(ProviderBuilder::new().connect(&rpc_url).await?);

    // Configuration addresses (replace with actual deployment addresses)
    let angstrom_address = address!("0x0000000aa232009084Bd71A5797d089AA4Edfad4");
    let controller_address = address!("0x1746484EA5e11C75e009252c102C8C33e0315fD4");
    let pool_manager_address = address!("0x000000000004444c5dc75cB358380D2e3dE08A90");
    let deploy_block = 22971782;

    // Connect to Angstrom WebSocket RPC for slot0 updates
    println!("🔌 Connecting to Angstrom WebSocket RPC...");
    let ws_url = std::env::var("ANGSTROM_WS_URL").expect("no angstrom ws set");

    let ws_client = Arc::new(WsClientBuilder::default().build(&ws_url).await?);
    let slot0_client = Slot0Client::new(ws_client);

    // Set up address book and pool registry
    println!("📡 Setting up address book and pool registry...");
    let address_book = L1AddressBook::new(controller_address, angstrom_address);
    let pool_registry = L1PoolRegistry::new(angstrom_address);
    let event_stream = NoOpEventStream::<Ethereum>::default();

    // Create pool manager service with both event stream and slot0 stream
    println!("🔨 Building pool manager service with slot0 stream...");
    let service = PoolManagerServiceBuilder::<_, _, _, NoOpSlot0Stream>::new(
        provider.clone(),
        address_book,
        pool_registry,
        pool_manager_address,
        deploy_block,
        event_stream
    )
    .with_slot0_stream(slot0_client)
    .build()
    .await?;

    println!("✅ Pool service initialized!");
    println!("📊 Found {} pools", service.get_pools().len());
    println!("🔗 Current block: {}", service.current_block());

    // Get all pool IDs to subscribe to slot0 updates
    let pool_ids: HashSet<PoolId> = service
        .get_pools()
        .iter()
        .map(|entry| *entry.key())
        .collect();
    println!("started with pool_ids {pool_ids:#?}");

    let pools = service.get_pools();
    // spawn the upkeeper service.
    tokio::spawn(service);

    // Main event loop - process both block events and slot0 updates
    println!("🔄 Starting event processing loop...");
    println!("   Block events: Pool creations, swaps, mints, burns");
    println!("   Slot0 updates: Real-time price, liquidity, and tick changes");
    println!("Press Ctrl+C to stop");

    loop {
        tokio::time::sleep(Duration::from_secs(12)).await;
        let updated_to_block = pools.get_block();
        tracing::info!("pools are updated to block number: {updated_to_block}");

        for id in &pool_ids {
            let pool = pools.get_pool(id).unwrap();
            let price = pool.current_price();

            // simulate a swap of a 4% price movment relative to sqrt_price.
            let price_limit = SqrtPriceX96::from(*price * U160::from(4) / U160::from(100));
            if let Ok(swap_output) = pool.swap_current_to_price(price_limit, false) {
                tracing::info!(pool_id=?id, t0=swap_output.total_d_t0, t1=swap_output.total_d_t1, price=?swap_output.end_price, "swapped to target price");
            }

            if let Ok(swap_output) =
                pool.swap_current_with_amount(I256::unchecked_from(10000), false, false)
            {
                tracing::info!(pool_id=?id, t0=swap_output.total_d_t0, t1=swap_output.total_d_t1, price=?swap_output.end_price, "swapped with amount");
            }
        }
    }
}
