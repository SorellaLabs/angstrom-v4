use std::{
    ops::Deref,
    sync::{Arc, atomic::AtomicU64}
};

use alloy_primitives::B256;
use dashmap::{DashMap, mapref::one::Ref};
use thiserror::Error;
use tokio::sync::{
    Notify,
    futures::{Notified, OwnedNotified}
};
use uni_v4_structure::{
    BaselinePoolState, PoolId, UpdatePool, V4Network, fee_config::FeeConfig,
    pool_updates::PoolUpdate
};
use uniswap_v3_math::error::UniswapV3MathError;

use crate::traits::{PoolUpdateDelivery, PoolUpdateDeliveryExt};

#[derive(Clone)]
pub struct UniswapPools<T: V4Network> {
    pools:           Arc<DashMap<PoolId, BaselinePoolState<T>>>,
    slot0_notifiers: Arc<DashMap<PoolId, Arc<Notify>>>,
    // what block these are up to date for.
    block_number:    Arc<AtomicU64>,
    // When the manager for the pools pushes a new block. It will notify all people who are
    // waiting.
    notifier:        Arc<Notify>
}

impl<T: V4Network> Deref for UniswapPools<T> {
    type Target = Arc<DashMap<PoolId, BaselinePoolState<T>>>;

    fn deref(&self) -> &Self::Target {
        &self.pools
    }
}

impl<T: V4Network> UniswapPools<T> {
    pub fn new(pools: Arc<DashMap<PoolId, BaselinePoolState<T>>>, block_number: u64) -> Self {
        Self {
            slot0_notifiers: Arc::new(
                pools
                    .iter()
                    .map(|pool| (*pool.key(), Arc::new(Notify::new())))
                    .collect()
            ),
            pools,
            block_number: Arc::new(AtomicU64::from(block_number)),
            notifier: Arc::new(Notify::new())
        }
    }

    pub fn get_block(&self) -> u64 {
        self.block_number.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub async fn wait_for_next_update(&self) {
        self.notifier.notified().await;
    }

    pub async fn wait_for_next_slot0_update(&self, pool_id: PoolId) {
        self.slot0_notifiers
            .get(&pool_id)
            .expect("slot0 notifier missing for pool_id — pool not yet registered")
            .notified()
            .await;
    }

    pub async fn notify_slot0_waiters(&self, pool_id: PoolId) {
        self.slot0_notifiers
            .get(&pool_id)
            .expect("slot0 notifier missing for pool_id — pool not yet registered")
            .notify_waiters();
    }

    pub fn get_pool(&self, pool_id: &PoolId) -> Option<Ref<'_, PoolId, BaselinePoolState<T>>> {
        self.pools.get(pool_id)
    }

    pub fn get_pools(&self) -> &DashMap<PoolId, BaselinePoolState<T>> {
        &self.pools
    }

    pub fn next_block_future(&self) -> Notified<'_> {
        self.notifier.notified()
    }

    pub fn next_block_future_owned(&self) -> OwnedNotified {
        self.notifier.clone().notified_owned()
    }

    pub async fn next_slot0_update_future_owned(&self, pool_id: PoolId) -> OwnedNotified {
        self.slot0_notifiers
            .get(&pool_id)
            .expect("slot0 notifier missing for pool_id — pool not yet registered")
            .clone()
            .notified_owned()
    }

    pub fn update_pools(&self, mut updates: Vec<PoolUpdate<T>>) {
        if updates.is_empty() {
            return;
        }

        let current_block_number = self.block_number.load(std::sync::atomic::Ordering::Relaxed);

        let mut new_block_number = None;
        // we sort ascending
        updates.sort_by(|a, b| a.sort(b));

        for update in updates {
            match update {
                PoolUpdate::NewBlock(block_number) => {
                    new_block_number = Some(block_number);
                }
                PoolUpdate::Reorg { to_block, .. } => {
                    new_block_number = Some(to_block);
                }
                PoolUpdate::SwapEvent { pool_id, event, .. } => {
                    let Some(mut pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };

                    let state = pool.value_mut();
                    // update slot0 values
                    state.update_slot0(event.tick, event.sqrt_price_x96.into(), event.liquidity);
                }
                PoolUpdate::LiquidityEvent { pool_id, event, .. } => {
                    let Some(mut pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };
                    let state = pool.value_mut();

                    state.update_liquidity(
                        event.tick_lower,
                        event.tick_upper,
                        event.liquidity_delta
                    );
                }
                PoolUpdate::FeeUpdate { pool_id, update, .. } => {
                    let Some(mut pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };
                    let fees = pool.value_mut().fees_mut();

                    fees.update_fees(update);
                }
                PoolUpdate::UpdatedSlot0 { pool_id, data } => {
                    let Some(mut pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };

                    let state = pool.value_mut();
                    state.update_slot0(data.tick, data.sqrt_price_x96.into(), data.liquidity);

                    if let Some(notifier) = self.slot0_notifiers.get(&pool_id) {
                        notifier.notify_waiters();
                    }
                }
                PoolUpdate::NewTicks { pool_id, ticks, tick_bitmap } => {
                    let Some(mut pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };

                    let baseline = pool.value_mut().get_baseline_liquidity_mut();

                    // Merge new ticks with existing ones
                    baseline.initialized_ticks_mut().extend(ticks);

                    // Update tick bitmap
                    for (word_pos, word) in tick_bitmap {
                        baseline.update_tick_bitmap(word_pos, word);
                    }
                }
                PoolUpdate::NewPoolState { pool_id, state } => {
                    self.pools.insert(pool_id, state);
                    self.slot0_notifiers
                        .insert(pool_id, Arc::new(Notify::new()));
                }
                PoolUpdate::ChainSpecific { pool_id, update } => {
                    if !update.valid_current_block(current_block_number) {
                        continue;
                    }

                    let Some(mut pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };

                    let should_notify = update.should_notify_waiters();

                    pool.update_chain_specific(update);

                    if should_notify && let Some(notifier) = self.slot0_notifiers.get(&pool_id) {
                        notifier.notify_waiters();
                    }
                }
            }
        }

        if let Some(bn) = new_block_number {
            self.block_number
                .store(bn, std::sync::atomic::Ordering::SeqCst);
            self.notifier.notify_waiters();
        }
    }

    /// Update pools using a PoolUpdateDelivery source
    /// Processes all available updates from the source
    pub fn update_from_source<D: PoolUpdateDelivery<T>>(&self, source: &mut D) {
        let mut updates = Vec::new();

        // Collect all available updates using the extension trait
        while let Some(update) = source.next_update() {
            updates.push(update);
        }

        // Process them using the existing method
        self.update_pools(updates);
    }

    /// Update pools by processing a single update from a PoolUpdateDelivery
    /// source Returns true if an update was processed, false if no updates
    /// were available
    pub fn update_single_from_source<D: PoolUpdateDelivery<T>>(&self, source: &mut D) -> bool {
        if let Some(update) = source.next_update() {
            self.update_pools(vec![update]);
            true
        } else {
            false
        }
    }
}

#[derive(Error, Debug)]
pub enum SwapSimulationError {
    #[error("Could not get next tick")]
    InvalidTick,
    #[error(transparent)]
    UniswapV3MathError(#[from] UniswapV3MathError),
    #[error("Liquidity underflow")]
    LiquidityUnderflow,
    #[error("Invalid sqrt price limit")]
    InvalidSqrtPriceLimit,
    #[error("Amount specified must be non-zero")]
    ZeroAmountSpecified
}

#[derive(Error, Debug)]
pub enum PoolError {
    #[error("Invalid signature: [{}]", .0.iter().map(|b| format!("0x{}", alloy_primitives::hex::encode(b))).collect::<Vec<_>>().join(", "))]
    InvalidEventSignature(Vec<B256>),
    #[error("Swap simulation failed")]
    SwapSimulationFailed,
    #[error("Pool already initialized")]
    PoolAlreadyInitialized,
    #[error("Pool is not initialized")]
    PoolNotInitialized,
    #[error(transparent)]
    SwapSimulationError(#[from] SwapSimulationError),
    #[error(transparent)]
    AlloyContractError(#[from] alloy_contract::Error),
    #[error(transparent)]
    AlloySolTypeError(#[from] alloy_sol_types::Error),
    #[error(transparent)]
    Eyre(#[from] eyre::Error)
}
