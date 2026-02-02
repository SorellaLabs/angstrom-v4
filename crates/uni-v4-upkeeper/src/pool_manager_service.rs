use std::{
    collections::HashSet,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll}
};

use alloy_network::Ethereum;
use alloy_primitives::Address;
use alloy_provider::Provider;
use futures::{Future, Stream, StreamExt};
use thiserror::Error;
use tokio::sync::mpsc;
use uni_v4_common::{PoolUpdate, UniswapPools, V4Network};
use uni_v4_structure::{
    BaselinePoolState, L1FeeConfiguration, PoolId, PoolKey,
    fee_config::FeeConfig,
    pool_registry::PoolRegistry,
    pool_updates::{L1PoolUpdate, Slot0Update}
};

use super::baseline_pool_factory::{BaselinePoolFactory, BaselinePoolFactoryError, UpdateMessage};
use crate::{
    pool_providers::{PoolEventStream, ProviderChainInitialization},
    slot0::Slot0Stream
};

/// Pool information combining BaselinePoolState with token metadata
#[derive(Debug, Clone)]
pub struct PoolInfo<T: V4Network> {
    pub baseline_state:  BaselinePoolState<T>,
    pub token0:          Address,
    pub token1:          Address,
    pub token0_decimals: u8,
    pub token1_decimals: u8
}

#[derive(Error, Debug)]
pub enum PoolManagerServiceError {
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Pool initialization error: {0}")]
    PoolInit(String),
    #[error("Pool factory error: {0}")]
    PoolFactory(String),
    #[error("Baseline pool factory error: {0}")]
    BaselineFactory(#[from] BaselinePoolFactoryError)
}

/// Service for managing Uniswap V4 pools with real-time block subscription
/// updates
pub struct PoolManagerService<P, T, Event, S = ()>
where
    P: Provider<T> + Unpin + Clone + 'static,
    T: V4Network,
    Event: PoolEventStream<T>
{
    pub(crate) factory:            BaselinePoolFactory<P, T>,
    pub(crate) event_stream:       Event,
    pub(crate) pools:              UniswapPools<T>,
    pub(crate) current_block:      u64,
    pub(crate) auto_pool_creation: bool,
    pub(crate) slot0_stream:       Option<S>,
    // If we are loading more ticks at a block, we will queue up updates messages here
    // so that we don't hit any race conditions.
    pending_updates:               Vec<PoolUpdate<T>>,
    // Channel for sending updates instead of applying them directly
    update_sender:                 Option<mpsc::Sender<PoolUpdate<T>>>
}

impl<P, T, Event, S> PoolManagerService<P, T, Event, S>
where
    P: Provider<T> + Clone + Unpin + 'static,
    T: V4Network,
    Event: PoolEventStream<T>,
    BaselinePoolFactory<P, T>: Stream<Item = UpdateMessage<T>> + Unpin,
    S: Slot0Stream,
    P: ProviderChainInitialization<T>,
    Self: PoolEventProcessor<T>
{
    /// Create a new PoolManagerService
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        provider: Arc<P>,
        event_stream: Event,
        address_book: T::AddressBook,
        pool_registry: T::PoolRegistry,
        pool_manager_address: Address,
        deploy_block: u64,
        tick_band: Option<u16>,
        tick_edge_threshold: Option<u16>,
        filter_pool_keys: Option<HashSet<PoolKey>>,
        auto_pool_creation: bool,
        slot0_stream: Option<S>,
        current_block: Option<u64>,
        ticks_per_batch: Option<usize>,
        update_channel: Option<mpsc::Sender<PoolUpdate<T>>>
    ) -> Result<Self, PoolManagerServiceError> {
        // Use provided current_block or get current block
        let current_block = if let Some(block) = current_block {
            block
        } else {
            provider.get_block_number().await.unwrap()
        };

        // Create factory with optional filtering
        let (factory, pools) = BaselinePoolFactory::new(
            deploy_block,
            current_block,
            address_book,
            pool_registry,
            provider.clone(),
            pool_manager_address,
            tick_band,
            tick_edge_threshold,
            filter_pool_keys,
            ticks_per_batch
        )
        .await;

        let mut service = Self {
            event_stream,
            factory,
            pools: UniswapPools::new(pools, current_block),
            current_block: deploy_block,
            auto_pool_creation,
            slot0_stream,
            pending_updates: Vec::new(),
            update_sender: update_channel
        };

        service
            .event_stream
            .set_pool_registry(service.factory.registry());

        // Ensure to register the pool_ids with the state stream.
        for pool_id in service.factory.registry().all_uniswap_pool_ids() {
            service.event_stream.start_tracking_pool(pool_id);
        }

        // Subscribe all initial pools to slot0 stream if present (using angstrom IDs)
        if let Some(slot0_stream) = &mut service.slot0_stream {
            let angstrom_pool_ids: HashSet<PoolId> =
                service.factory.registry().all_angstrom_pool_ids().collect();
            slot0_stream.subscribe_pools(angstrom_pool_ids);
        }

        // Send all initialized pools through the channel on startup
        if service.update_sender.is_some() {
            let initial_pool_updates: Vec<PoolUpdate<T>> = service
                .pools
                .get_pools()
                .iter()
                .map(|entry| PoolUpdate::NewPoolState {
                    pool_id: *entry.key(),
                    state:   entry.value().clone()
                })
                .collect();

            for update in initial_pool_updates {
                service.dispatch_update(update);
            }
        }

        Ok(service)
    }

    /// Get all currently tracked pools
    pub fn get_pools(&self) -> UniswapPools<T> {
        self.pools.clone()
    }

    /// Get the current block number being processed
    pub fn current_block(&self) -> u64 {
        self.current_block
    }

    /// Gives a reference to the optional slot0 stream
    pub fn slot0_stream_ref(&self) -> Option<&S> {
        self.slot0_stream.as_ref()
    }

    /// Handle a new pool creation
    pub(crate) fn handle_new_pool(
        &mut self,
        pool_key: PoolKey,
        block_number: u64,
        fee_cfg: T::FeeConfig
    ) {
        self.factory
            .queue_pool_creation(pool_key, block_number, fee_cfg);
    }

    /// Dispatch an update either via channel or apply directly
    fn dispatch_update(&mut self, update: PoolUpdate<T>) {
        if let Some(sender) = &self.update_sender {
            // Channel mode: send the update
            if let Err(e) = sender.try_send(update.clone()) {
                tracing::error!("Failed to send update via channel: {}", e);
            }

            // Always process certain critical updates internally even in channel mode
            match &update {
                PoolUpdate::NewBlock(block) => {
                    self.current_block = *block;
                }
                PoolUpdate::ChainSpecific { pool_id, update } => {
                    self.dispath_chain_specific_update(*pool_id, update.clone());
                }
                _ => {
                    // Other updates are just forwarded via channel without
                    // internal processing
                }
            }
        } else {
            self.process_pool_update(update.clone());
            self.pools.update_pools(vec![update]);
        }
    }

    /// Process a pool update event from the PoolUpdateProvider
    pub fn process_pool_update(&mut self, update: PoolUpdate<T>) {
        match &update {
            PoolUpdate::NewBlock(block_number) => {
                self.current_block = *block_number;
            }
            PoolUpdate::SwapEvent { pool_id, event, .. } => {
                tracing::debug!("Swap event for pool {:?}: {:?}", pool_id, event);
            }
            PoolUpdate::LiquidityEvent { pool_id, event, .. } => {
                tracing::debug!("Liquidity event for pool {:?}: {:?}", pool_id, event);
            }
            PoolUpdate::ChainSpecific { pool_id, update } => {
                self.handle_chain_specific_update(*pool_id, update);
            }

            PoolUpdate::FeeUpdate { pool_id, update, .. } => {
                if let Some(mut pool) = self.pools.get_pools().get_mut(pool_id) {
                    let fees = pool.fees_mut();
                    fees.update_fees(*update);

                    tracing::info!("Updated fees for pool {pool_id:?}:\n{update:?}",);
                } else {
                    tracing::warn!("Received fee update for unknown pool: {:?}", pool_id);
                }
            }
            PoolUpdate::UpdatedSlot0 { pool_id, data } => {
                tracing::debug!("Updated slot0 for pool {:?}: {:?}", pool_id, data);
            }
            PoolUpdate::Reorg { from_block, to_block } => {
                tracing::warn!("Reorg detected from block {} to {}", from_block, to_block);
            }
            PoolUpdate::NewPoolState { pool_id, state: _ } => {
                // This comes from the factory - just track the pool
                self.event_stream.start_tracking_pool(*pool_id);

                // Subscribe new pool to slot0 stream (using angstrom ID)
                if let Some(slot0_stream) = &mut self.slot0_stream
                    && let Some(angstrom_pool_id) = self
                        .factory
                        .registry()
                        .angstrom_pool_id_from_uniswap_pool_id(*pool_id)
                {
                    slot0_stream.subscribe_pools(HashSet::from([angstrom_pool_id]));
                }

                tracing::info!("Tracking new pool from factory: {:?}", pool_id);
            }
            PoolUpdate::NewTicks { .. } => {
                // These are handled by update_pools
                tracing::debug!("NewTicks update will be handled by update_pools");
            }
        }
    }
}

impl<P, T, Event, S> Future for PoolManagerService<P, T, Event, S>
where
    P: Provider<T> + Clone + Unpin + 'static,
    T: V4Network,
    Event: PoolEventStream<T>,
    BaselinePoolFactory<P, T>: Stream<Item = UpdateMessage<T>> + Unpin,
    S: Slot0Stream,
    P: ProviderChainInitialization<T>,
    Self: PoolEventProcessor<T>
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Continuously poll the factory stream
        let this = self.get_mut();
        match this.factory.poll_next_unpin(cx) {
            Poll::Ready(Some(update)) => {
                // Convert factory update to PoolUpdate and dispatch
                let pool_update = match update {
                    UpdateMessage::NewTicks(pool_id, ticks, tick_bitmap) => {
                        PoolUpdate::NewTicks { pool_id, ticks, tick_bitmap }
                    }
                    UpdateMessage::NewPool(pool_id, state) => {
                        PoolUpdate::NewPoolState { pool_id, state }
                    }
                };
                this.dispatch_update(pool_update);
            }
            Poll::Ready(None) => {
                // Stream ended, which shouldn't happen in our case.
                return Poll::Ready(());
            }
            _ => {}
        }

        if !this.factory.is_processing() {
            let updates = this.pending_updates.drain(..).collect::<Vec<_>>();

            if this.update_sender.is_some() {
                // Channel mode: dispatch each update
                for event in updates {
                    this.dispatch_update(event);
                }
            } else {
                // Direct mode: apply updates and check tick ranges
                this.pools.update_pools(updates.clone());
                for event in updates {
                    this.process_pool_update(event);
                }

                // Check tick ranges for all pools after updates
                for entry in this.pools.get_pools().iter() {
                    this.factory.check_and_request_ticks_if_needed(
                        *entry.key(),
                        entry.value(),
                        Some(this.current_block)
                    );
                }
            }
        }

        while let Poll::Ready(events) = this.event_stream.poll_next_unpin(cx) {
            if let Some(events) = events {
                if this.factory.is_processing() {
                    this.pending_updates.extend(events);
                    continue;
                }

                if this.update_sender.is_some() {
                    // Channel mode: dispatch each update
                    for event in events {
                        this.dispatch_update(event);
                    }
                } else {
                    // Direct mode: apply updates and check tick ranges
                    this.pools.update_pools(events.clone());
                    for event in events {
                        this.process_pool_update(event);
                    }
                    // Check tick ranges for all pools after updates
                    for entry in this.pools.get_pools().iter() {
                        this.factory.check_and_request_ticks_if_needed(
                            *entry.key(),
                            entry.value(),
                            Some(this.current_block)
                        );
                    }
                }
            } else {
                return Poll::Ready(());
            }
        }

        if let Some(slot0_stream) = this.slot0_stream.as_mut() {
            let mut slot0_updates = Vec::new();
            while let Poll::Ready(Some(update)) = slot0_stream.poll_next_unpin(cx) {
                slot0_updates.push(update);
            }
            this.handle_slot0_updates(slot0_updates);
        }

        Poll::Pending
    }
}

pub trait PoolEventProcessor<T: V4Network> {
    fn handle_chain_specific_update(&mut self, pool_id: PoolId, update: &T::PoolUpdate);

    fn handle_slot0_updates(&mut self, slot0_updates: Vec<Slot0Update>);

    fn dispath_chain_specific_update(&mut self, pool_id: PoolId, update: T::PoolUpdate);
}

impl<P, Event, S> PoolEventProcessor<Ethereum> for PoolManagerService<P, Ethereum, Event, S>
where
    P: Provider<Ethereum> + Clone + Unpin + 'static,
    Event: PoolEventStream<Ethereum>,
    BaselinePoolFactory<P, Ethereum>: Stream<Item = UpdateMessage<Ethereum>> + Unpin,
    S: Slot0Stream,
    P: ProviderChainInitialization<Ethereum>
{
    fn handle_chain_specific_update(&mut self, _: PoolId, update: &L1PoolUpdate) {
        match update {
            L1PoolUpdate::NewPool {
                pool_id,
                bundle_fee,
                swap_fee,
                protocol_fee,
                tick_spacing,
                block,
                ..
            } => {
                if self.auto_pool_creation {
                    // Reconstruct pool_key from the NewPool data
                    // We need to get the pool_key from the registry
                    if let Some(pool_key) = self.factory.registry().get(pool_id) {
                        self.handle_new_pool(
                            *pool_key,
                            *block,
                            L1FeeConfiguration {
                                bundle_fee:   *bundle_fee,
                                swap_fee:     *swap_fee,
                                protocol_fee: *protocol_fee
                            }
                        );

                        tracing::info!(
                            "Pool configured: {:?}, bundle_fee: {}, swap_fee: {}, protocol_fee: \
                             {}, tick_spacing: {}",
                            pool_id,
                            bundle_fee,
                            swap_fee,
                            protocol_fee,
                            tick_spacing
                        );
                    } else {
                        tracing::warn!("Pool {:?} not found in registry", pool_id);
                    }
                } else {
                    tracing::info!(
                        "Ignoring pool configured event (auto creation disabled): {:?}",
                        pool_id
                    );
                }
            }
            L1PoolUpdate::PoolRemoved { pool_id, .. } => {
                tracing::info!("Pool removed: {:?}", pool_id);
                self.pools.remove(pool_id);
                self.factory.remove_pool_by_id(*pool_id);

                // Unsubscribe pool from slot0 stream (pool_id here is already angstrom ID)
                if let Some(slot0_stream) = &mut self.slot0_stream {
                    slot0_stream.unsubscribe_pools(HashSet::from([*pool_id]));
                }
            }
            L1PoolUpdate::Slot0Update(_) => {}
        }
    }

    fn handle_slot0_updates(&mut self, slot0_updates: Vec<Slot0Update>) {
        for update in slot0_updates {
            let pool_update = PoolUpdate::ChainSpecific {
                pool_id: update.uni_pool_id,
                update:  L1PoolUpdate::Slot0Update(update)
            };
            self.dispatch_update(pool_update);
        }
    }

    fn dispath_chain_specific_update(&mut self, pool_id: PoolId, update: L1PoolUpdate) {
        match update {
            L1PoolUpdate::NewPool { .. } => {
                // CRITICAL: Process new pool to ensure it gets created in the factory
                // This will trigger pool data loading and initialization
                self.process_pool_update(PoolUpdate::ChainSpecific { pool_id, update });
            }
            L1PoolUpdate::PoolRemoved { .. } => {
                let update = PoolUpdate::ChainSpecific { pool_id, update };
                // Process pool removal to clean up internal state
                self.pools.update_pools(vec![update.clone()]);
                self.process_pool_update(update);
            }
            _ => ()
        }
    }
}
