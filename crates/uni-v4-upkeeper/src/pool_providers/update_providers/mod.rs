pub mod l1;
pub mod l2;

use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll}
};

use alloy_consensus::BlockHeader;
use alloy_eips::BlockId;
use alloy_network::BlockResponse;
use alloy_primitives::{Address, U160};
use alloy_provider::Provider;
use alloy_rpc_types::{Block, Filter};
use alloy_sol_types::SolEvent;
use futures::{FutureExt, StreamExt, stream::Stream};
use thiserror::Error;
use uni_v4_common::{ModifyLiquidityEventData, PoolUpdate, StreamMode, SwapEventData, V4Network};
use uni_v4_structure::{PoolId, UpdatePool, pool_registry::PoolRegistry, updates::Slot0Data};

use crate::{
    pool_data_loader::{DataLoader, IUniswapV4Pool, PoolDataLoader},
    pool_providers::{PoolEventStream, ProviderChainUpdate}
};

/// Default number of blocks to keep in history for reorg detection
const DEFAULT_REORG_DETECTION_BLOCKS: u64 = 10;

/// Default chunk size for block processing
const DEFAULT_REORG_LOOKBACK_BLOCK_CHUNK: u64 = 100;

#[derive(Debug, Error)]
pub enum PoolUpdateError {
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Event decode error: {0}")]
    EventDecode(String),
    #[error("Reorg handling error: {0}")]
    ReorgHandling(String)
}

/// Stored event for reorg handling - only liquidity events need to be stored
#[derive(Debug, Clone)]
struct StoredEvent {
    block:           u64,
    tx_index:        u64,
    log_index:       u64,
    pool_id:         PoolId,
    liquidity_event: ModifyLiquidityEventData
}

/// Pool update provider that streams pool state changes
pub struct PoolUpdateProvider<P, T>
where
    P: Provider<T> + 'static,
    T: V4Network
{
    provider:                   Arc<P>,
    pool_manager:               Address,
    address_book:               T::AddressBook,
    pool_registry:              T::PoolRegistry,
    tracked_pools:              HashSet<PoolId>,
    event_history:              VecDeque<StoredEvent>,
    current_block:              u64,
    reorg_detection_blocks:     u64,
    reorg_lookback_block_chunk: u64,
    stream_mode:                StreamMode
}

impl<P, T> PoolUpdateProvider<P, T>
where
    P: Provider<T> + 'static,
    T: V4Network,
    Self: ProviderChainUpdate<T>
{
    /// Create a new pool update provider
    pub async fn new(
        provider: Arc<P>,
        pool_manager: Address,
        address_book: T::AddressBook,
        pool_registry: T::PoolRegistry
    ) -> Self {
        let current_block = provider
            .get_block(BlockId::Number(alloy_eips::BlockNumberOrTag::Latest))
            .await
            .unwrap()
            .unwrap()
            .header()
            .number();

        Self::new_at_block(provider, pool_manager, address_book, pool_registry, current_block)
    }

    /// Create a new pool update provider at a specific block
    pub fn new_at_block(
        provider: Arc<P>,
        pool_manager: Address,
        address_book: T::AddressBook,
        pool_registry: T::PoolRegistry,
        current_block: u64
    ) -> Self {
        Self::new_with_config(
            provider,
            pool_manager,
            current_block,
            DEFAULT_REORG_DETECTION_BLOCKS,
            DEFAULT_REORG_LOOKBACK_BLOCK_CHUNK,
            address_book,
            pool_registry
        )
    }

    /// Create a new pool update provider with custom configuration
    pub fn new_with_config(
        provider: Arc<P>,
        pool_manager: Address,
        current_block: u64,
        reorg_detection_blocks: u64,
        reorg_lookback_block_chunk: u64,
        address_book: T::AddressBook,
        pool_registry: T::PoolRegistry
    ) -> Self {
        Self {
            provider,
            pool_manager,
            tracked_pools: HashSet::new(),
            event_history: VecDeque::with_capacity(reorg_detection_blocks as usize),
            current_block,
            reorg_detection_blocks,
            reorg_lookback_block_chunk,
            stream_mode: StreamMode::default(),
            address_book,
            pool_registry
        }
    }

    pub fn address_book(&self) -> T::AddressBook {
        self.address_book
    }

    pub fn with_address_book(mut self, address_book: T::AddressBook) -> Self {
        self.address_book = address_book;
        self
    }

    /// Set the stream mode for this provider
    pub fn with_stream_mode(mut self, mode: StreamMode) -> Self {
        self.stream_mode = mode;
        self
    }

    /// Add a pool to track
    pub fn add_pool(&mut self, pool_id: PoolId) {
        self.tracked_pools.insert(pool_id);
    }

    /// Remove a pool from tracking
    pub fn remove_pool(&mut self, pool_id: PoolId) {
        self.tracked_pools.remove(&pool_id);
    }

    /// Get all tracked pool IDs
    pub fn tracked_pools(&self) -> Vec<PoolId> {
        self.tracked_pools.iter().copied().collect()
    }

    /// Process a swap event log
    fn process_swap_event(
        &self,
        log: &alloy_rpc_types::Log,
        block_number: u64
    ) -> Option<PoolUpdate<T>> {
        if let Ok(swap_event) = IUniswapV4Pool::Swap::decode_log(&log.inner) {
            // Check if we're tracking this Uniswap pool ID
            if self.tracked_pools.contains(&swap_event.id) {
                let event_data = SwapEventData {
                    sender:         swap_event.sender,
                    amount0:        swap_event.amount0,
                    amount1:        swap_event.amount1,
                    sqrt_price_x96: swap_event.sqrtPriceX96,
                    liquidity:      swap_event.liquidity,
                    tick:           swap_event.tick.as_i32(),
                    fee:            swap_event.fee.to()
                };

                return Some(PoolUpdate::SwapEvent {
                    pool_id:   swap_event.id, // Use Uniswap pool ID
                    block:     block_number,
                    tx_index:  log.transaction_index.unwrap(),
                    log_index: log.log_index.unwrap(),
                    event:     event_data
                });
            }
        }
        None
    }

    /// Process a liquidity event log
    fn process_liquidity_event(
        &mut self,
        log: &alloy_rpc_types::Log,
        block_number: u64,
        store_in_history: bool
    ) -> Option<PoolUpdate<T>> {
        if let Ok(modify_event) = IUniswapV4Pool::ModifyLiquidity::decode_log(&log.inner) {
            // Check if we're tracking this Uniswap pool ID
            if self.tracked_pools.contains(&modify_event.id) {
                let event_data = ModifyLiquidityEventData {
                    sender:          modify_event.sender,
                    tick_lower:      modify_event.tickLower.as_i32(),
                    tick_upper:      modify_event.tickUpper.as_i32(),
                    liquidity_delta: modify_event.liquidityDelta,
                    salt:            modify_event.salt.0
                };

                // Store in history only if requested
                if store_in_history {
                    self.add_to_history(StoredEvent {
                        block:           block_number,
                        tx_index:        log.transaction_index.unwrap(),
                        log_index:       log.log_index.unwrap(),
                        pool_id:         modify_event.id, // Use Uniswap pool ID
                        liquidity_event: event_data.clone()
                    });
                }

                return Some(PoolUpdate::LiquidityEvent {
                    pool_id:   modify_event.id, // Use Uniswap pool ID
                    block:     block_number,
                    tx_index:  log.transaction_index.unwrap(),
                    log_index: log.log_index.unwrap(),
                    event:     event_data
                });
            }
        }
        None
    }

    /// Process events for a block range
    async fn process_events_for_block_range(
        &mut self,
        from_block: u64,
        to_block: u64,
        store_in_history: bool
    ) -> Result<Vec<PoolUpdate<T>>, PoolUpdateError> {
        let mut updates = Vec::new();

        // If no pools are tracked, return early
        if self.tracked_pools.is_empty() {
            return Ok(updates);
        }

        // Create pool topics for filtering - tracked_pools already contains Uniswap
        // pool IDs
        let pool_topics: Vec<_> = self
            .tracked_pools
            .iter()
            .map(|pool_id| pool_id.0.into())
            .collect();

        // Create filters for swap and liquidity events
        let swap_filter = Filter::new()
            .address(self.pool_manager)
            .event_signature(IUniswapV4Pool::Swap::SIGNATURE_HASH)
            .topic1(pool_topics.clone())
            .from_block(from_block)
            .to_block(to_block);

        let modify_filter = Filter::new()
            .address(self.pool_manager)
            .event_signature(IUniswapV4Pool::ModifyLiquidity::SIGNATURE_HASH)
            .topic1(pool_topics)
            .from_block(from_block)
            .to_block(to_block);

        // Get logs for both event types
        let (swap_logs, modify_logs) = futures::try_join!(
            self.provider.get_logs(&swap_filter),
            self.provider.get_logs(&modify_filter)
        )
        .map_err(|e| PoolUpdateError::Provider(format!("Failed to get logs: {e}")))?;

        // Process swap logs
        for log in swap_logs {
            let block_number = log.block_number.unwrap_or(from_block);
            if let Some(update) = self.process_swap_event(&log, block_number) {
                updates.push(update);
            }
        }

        // Process modify liquidity logs
        for log in modify_logs {
            let block_number = log.block_number.unwrap_or(from_block);
            if let Some(update) = self.process_liquidity_event(&log, block_number, store_in_history)
            {
                updates.push(update);
            }
        }

        // Process chain specific data
        let chain_specific_logs = self.fetch_chain_data(from_block, to_block).await?;
        updates.extend(chain_specific_logs);

        Ok(updates)
    }

    /// Process events for a specific block
    async fn process_block_events(
        &mut self,
        block_number: u64
    ) -> Result<Vec<PoolUpdate<T>>, PoolUpdateError> {
        // Use the shared helper with store_in_history = true for single blocks
        self.process_events_for_block_range(block_number, block_number, true)
            .await
    }

    /// Add event to history, maintaining the 10-block window
    fn add_to_history(&mut self, event: StoredEvent) {
        self.event_history.push_back(event);

        // Maintain exactly reorg_detection_blocks worth of history
        // Remove all events from blocks that are too old
        let cutoff_block = self
            .current_block
            .saturating_sub(self.reorg_detection_blocks - 1);

        // Remove all events from blocks older than cutoff
        self.event_history.retain(|e| e.block >= cutoff_block);
    }

    /// Fetch current slot0 data for a pool at the current block
    async fn fetch_slot0_data(&self, pool_id: PoolId) -> Result<Slot0Data, PoolUpdateError> {
        self.fetch_slot0_data_at_block(pool_id, self.current_block)
            .await
    }

    /// Fetch slot0 data for a pool at a specific block
    async fn fetch_slot0_data_at_block(
        &self,
        pool_id: PoolId,
        block: u64
    ) -> Result<Slot0Data, PoolUpdateError> {
        // Get the internal pool ID from the conversion map
        let pool_id_set = self
            .pool_registry
            .make_pool_id_set(pool_id)
            .ok_or_else(|| {
                PoolUpdateError::Provider(format!("Pool ID {pool_id:?} not found in registry"))
            })?;

        // Create a DataLoader for this pool
        let data_loader = DataLoader::new_with_registry(
            pool_id_set,
            self.pool_registry.clone(),
            self.pool_manager
        );

        // Load pool data at specific block
        let pool_data = data_loader
            .load_pool_data(Some(block), self.provider.clone())
            .await
            .map_err(|e| PoolUpdateError::Provider(format!("Failed to load pool data: {e}")))?;

        Ok(Slot0Data {
            sqrt_price_x96: U160::from(pool_data.sqrtPrice),
            tick:           pool_data.tick.as_i32(),
            liquidity:      pool_data.liquidity
        })
    }

    /// Backfill events for missed blocks
    async fn backfill_blocks(
        &mut self,
        from_block: u64,
        to_block: u64
    ) -> Result<Vec<PoolUpdate<T>>, PoolUpdateError> {
        let mut all_updates = Vec::new();

        // Process blocks in chunks to avoid overwhelming the provider
        let mut current = from_block;

        while current <= to_block {
            let end = (current + self.reorg_lookback_block_chunk - 1).min(to_block);

            // Use the shared helper with store_in_history = false for backfilling
            let chunk_updates = self
                .process_events_for_block_range(current, end, false)
                .await?;
            all_updates.extend(chunk_updates);

            current = end + 1;
        }

        Ok(all_updates)
    }

    /// Get inverse liquidity events for reorg handling
    fn get_inverse_liquidity_events(&self, from_block: u64, to_block: u64) -> Vec<PoolUpdate<T>> {
        let mut inverse_events = Vec::new();

        // Iterate through history in reverse order to process most recent first
        for event in self.event_history.iter().rev() {
            if event.block < from_block || event.block > to_block {
                continue;
            }

            // Create inverse event by negating liquidity delta
            let inverse_event = ModifyLiquidityEventData {
                sender:          event.liquidity_event.sender,
                tick_lower:      event.liquidity_event.tick_lower,
                tick_upper:      event.liquidity_event.tick_upper,
                liquidity_delta: -event.liquidity_event.liquidity_delta,
                salt:            event.liquidity_event.salt
            };

            inverse_events.push(PoolUpdate::LiquidityEvent {
                pool_id:   event.pool_id,
                block:     event.block,
                tx_index:  event.tx_index,
                log_index: event.log_index,
                event:     inverse_event
            });
        }

        inverse_events
    }

    /// Get pools affected by events
    fn get_affected_pools(&self, updates: &[PoolUpdate<T>]) -> HashSet<PoolId> {
        let mut affected_pools = HashSet::new();

        for update in updates {
            match update {
                PoolUpdate::SwapEvent { pool_id, .. }
                | PoolUpdate::LiquidityEvent { pool_id, .. }
                | PoolUpdate::UpdatedSlot0 { pool_id, .. }
                | PoolUpdate::FeeUpdate { pool_id, .. } => {
                    affected_pools.insert(*pool_id);
                }
                PoolUpdate::ChainSpecific { pool_id, update } => {
                    if update.is_pool_affected() {
                        affected_pools.insert(*pool_id);
                    }
                }
                _ => {}
            }
        }

        affected_pools
    }

    /// Clear history for reorg
    fn clear_history_from_block(&mut self, from_block: u64) {
        self.event_history.retain(|event| event.block < from_block);
    }

    /// Handle a reorg event
    async fn handle_reorg(&mut self) -> Vec<PoolUpdate<T>> {
        let mut updates = Vec::new();
        let reorg_start = self
            .current_block
            .saturating_sub(self.reorg_detection_blocks - 1);

        // 1. First, emit the reorg event so the pipeline knows a reorg is happening
        updates.push(PoolUpdate::Reorg { from_block: reorg_start, to_block: self.current_block });

        // 2. Get inverse liquidity events
        let inverse_events = self.get_inverse_liquidity_events(reorg_start, self.current_block);

        // Filter inverse events based on stream mode
        match self.stream_mode {
            StreamMode::Full => {
                updates.extend(inverse_events.clone());
            }
            StreamMode::InitializationOnly => {
                // In InitializationOnly mode, we don't need inverse liquidity
                // events as we're not tracking swap/liquidity
                // changes
            }
        }

        // 3. Clear affected history
        self.clear_history_from_block(reorg_start);

        // 4. Re-query the blocks
        match self.backfill_blocks(reorg_start, self.current_block).await {
            Ok(fresh_events) => {
                // Get affected pools from both inverse and fresh events
                let mut affected_pools = self.get_affected_pools(&inverse_events);
                affected_pools.extend(self.get_affected_pools(&fresh_events));

                // Add fresh events to history
                for update in &fresh_events {
                    if let Some(stored_event) = Self::update_to_stored_event(update) {
                        self.add_to_history(stored_event);
                    }
                }

                // Filter fresh events based on stream mode
                match self.stream_mode {
                    StreamMode::Full => {
                        updates.extend(fresh_events);
                    }
                    StreamMode::InitializationOnly => {
                        // Only include initialization-related updates
                        updates.extend(fresh_events.into_iter().filter(|update| match update {
                            PoolUpdate::FeeUpdate { .. }
                            | PoolUpdate::UpdatedSlot0 { .. }
                            | PoolUpdate::NewPoolState { .. } => true,
                            PoolUpdate::ChainSpecific { pool_id: _, update } => {
                                update.is_initialization_event()
                            }
                            _ => false
                        }));
                    }
                }

                // 5. Query slot0 for affected pools
                for pool_id in affected_pools {
                    if let Ok(slot0_data) = self.fetch_slot0_data(pool_id).await {
                        updates.push(PoolUpdate::UpdatedSlot0 { pool_id, data: slot0_data });
                    }
                }
            }
            Err(e) => {
                // Log error but continue
                panic!("Failed to backfill during reorg: {e}");
            }
        }
        updates.push(PoolUpdate::Reorg { from_block: reorg_start, to_block: self.current_block });

        updates
    }

    pub async fn on_new_block(&mut self, block: Block) -> Vec<PoolUpdate<T>> {
        let mut updates = Vec::new();
        let block_number = block.number();

        // Check for reorg
        if block_number == self.current_block {
            // Reorg detected!
            updates = self.handle_reorg().await;
        } else if block_number > self.current_block {
            // Always emit NewBlock event first for normal block progression
            updates.push(PoolUpdate::NewBlock(block_number));

            // Then process block events
            match self.process_block_events(block_number).await {
                Ok(block_updates) => {
                    // Filter updates based on stream mode
                    match self.stream_mode {
                        StreamMode::Full => {
                            // Include all updates
                            updates.extend(block_updates);
                        }
                        StreamMode::InitializationOnly => {
                            // Only include initialization-related updates
                            updates.extend(block_updates.into_iter().filter(
                                |update| match update {
                                    PoolUpdate::FeeUpdate { .. }
                                    | PoolUpdate::UpdatedSlot0 { .. }
                                    | PoolUpdate::NewPoolState { .. } => true,
                                    PoolUpdate::ChainSpecific { pool_id: _, update } => {
                                        update.is_initialization_event()
                                    }
                                    _ => false
                                }
                            ));
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to process block {}: {}", block_number, e);
                }
            }

            // Update current block
            self.current_block = block_number;

            // Clean up old events from history to maintain exactly reorg_detection_blocks
            let cutoff_block = self
                .current_block
                .saturating_sub(self.reorg_detection_blocks - 1);
            self.event_history.retain(|e| e.block >= cutoff_block);
        } else if block_number < self.current_block {
            // Block is behind our current block, this shouldn't happen in normal operation
            tracing::warn!(
                "Received old block {} when current block is {}",
                block_number,
                self.current_block
            );
        }

        updates
    }

    /// Convert PoolUpdate to StoredEvent for history
    /// Only liquidity events are stored since we re-query slot0 after reorgs
    fn update_to_stored_event(update: &PoolUpdate<T>) -> Option<StoredEvent> {
        match update {
            PoolUpdate::LiquidityEvent { pool_id, block, tx_index, log_index, event } => {
                Some(StoredEvent {
                    block:           *block,
                    tx_index:        *tx_index,
                    log_index:       *log_index,
                    pool_id:         *pool_id,
                    liquidity_event: event.clone()
                })
            }
            _ => None
        }
    }
}

pub struct StateStream<P, T, B>
where
    P: Provider<T> + 'static,
    T: V4Network,
    B: Stream<Item = Block> + Unpin + Send + 'static,
    PoolUpdateProvider<P, T>: ProviderChainUpdate<T>
{
    update_provider:      Option<PoolUpdateProvider<P, T>>,
    block_stream:         B,
    processing: Option<
        Pin<Box<dyn Future<Output = (PoolUpdateProvider<P, T>, Vec<PoolUpdate<T>>)> + Send>>
    >,
    start_tracking_pools: Vec<PoolId>,
    stop_tracking_pools:  Vec<PoolId>,
    pool_reg:             Option<T::PoolRegistry>
}

impl<P, T, B> StateStream<P, T, B>
where
    P: Provider<T> + 'static,
    T: V4Network,
    B: Stream<Item = Block> + Unpin + Send + 'static,
    PoolUpdateProvider<P, T>: ProviderChainUpdate<T>
{
    pub fn new(update_provider: PoolUpdateProvider<P, T>, block_stream: B) -> Self {
        Self {
            update_provider: Some(update_provider),
            block_stream,
            processing: None,
            start_tracking_pools: vec![],
            stop_tracking_pools: vec![],
            pool_reg: None
        }
    }
}

impl<P, T, B> PoolEventStream<T> for StateStream<P, T, B>
where
    P: Provider<T> + 'static,
    T: V4Network,
    B: Stream<Item = Block> + Unpin + Send + 'static,
    PoolUpdateProvider<P, T>: ProviderChainUpdate<T>
{
    fn stop_tracking_pool(&mut self, pool_id: PoolId) {
        if let Some(update_provider) = self.update_provider.as_mut() {
            update_provider.remove_pool(pool_id);
        } else {
            self.stop_tracking_pools.push(pool_id);
        }
    }

    fn start_tracking_pool(&mut self, pool_id: PoolId) {
        if let Some(update_provider) = self.update_provider.as_mut() {
            update_provider.add_pool(pool_id);
        } else {
            self.start_tracking_pools.push(pool_id);
        }
    }

    fn set_pool_registry(&mut self, pool_registry: T::PoolRegistry) {
        if let Some(update_provider) = self.update_provider.as_mut() {
            update_provider.pool_registry = pool_registry;
        } else {
            self.pool_reg = Some(pool_registry);
        }
    }
}

impl<P, T, B> Stream for StateStream<P, T, B>
where
    P: Provider<T> + 'static,
    T: V4Network,
    B: Stream<Item = Block> + Unpin + Send + 'static,
    PoolUpdateProvider<P, T>: ProviderChainUpdate<T>
{
    type Item = Vec<PoolUpdate<T>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // If we are processing something, we don't want to poll the block stream as
        // this could cause panics as the update provider has moved.
        if let Some(mut processing) = this.processing.take() {
            if let Poll::Ready((provider, new_updates)) = processing.poll_unpin(cx) {
                this.update_provider = Some(provider);

                return Poll::Ready(Some(new_updates));
            }
            this.processing = Some(processing);

            return Poll::Pending
        }

        let updater = this.update_provider.as_mut().unwrap();
        for pool in this.start_tracking_pools.drain(..) {
            updater.add_pool(pool);
        }
        for pool in this.stop_tracking_pools.drain(..) {
            updater.remove_pool(pool);
        }
        if let Some(pool_reg) = this.pool_reg.take() {
            updater.pool_registry = pool_reg;
        }

        if let Poll::Ready(possible_new_block) = this.block_stream.poll_next_unpin(cx) {
            if let Some(new_block) = possible_new_block {
                cx.waker().wake_by_ref();
                let mut update_provider = this.update_provider.take().unwrap();

                let processing_future = async move {
                    let updates = update_provider.on_new_block(new_block).await;
                    (update_provider, updates)
                }
                .boxed();

                this.processing = Some(processing_future)
            } else {
                return Poll::Ready(None)
            }
        }

        Poll::Pending
    }
}
