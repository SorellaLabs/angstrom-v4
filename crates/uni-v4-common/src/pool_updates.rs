use std::collections::{HashMap, VecDeque};

use alloy_primitives::U256;
use uni_v4_structure::{
    BaselinePoolState, PoolId,
    fee_config::FeeConfig,
    pool_updates::{ModifyLiquidityEventData, PoolUpdate, Slot0Data, SwapEventData},
    tick_info::TickInfo
};

use crate::{V4Network, traits::PoolUpdateDelivery};

/// A queue-based implementation of PoolUpdateDelivery that allows feeding
/// PoolUpdate instances
pub struct PoolUpdateQueue<T: V4Network> {
    updates: VecDeque<PoolUpdate<T>>
}

impl<T: V4Network> PoolUpdateQueue<T> {
    /// Create a new empty PoolUpdateQueue
    pub fn new() -> Self {
        Self { updates: VecDeque::new() }
    }

    /// Add a single update to the queue
    pub fn push(&mut self, update: PoolUpdate<T>) {
        self.updates.push_back(update);
    }

    /// Add multiple updates to the queue
    pub fn extend(&mut self, updates: impl IntoIterator<Item = PoolUpdate<T>>) {
        self.updates.extend(updates);
    }

    /// Get the number of pending updates
    pub fn len(&self) -> usize {
        self.updates.len()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }

    /// Clear all pending updates
    pub fn clear(&mut self) {
        self.updates.clear();
    }
}

impl<T: V4Network> Default for PoolUpdateQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: V4Network> PoolUpdateDelivery<T> for PoolUpdateQueue<T> {
    fn get_new_block(&mut self) -> Option<u64> {
        match self.updates.front() {
            Some(PoolUpdate::NewBlock(block)) => {
                let block = *block;
                self.updates.pop_front();
                Some(block)
            }
            _ => None
        }
    }

    fn get_reorg(&mut self) -> Option<(u64, u64)> {
        match self.updates.front() {
            Some(PoolUpdate::Reorg { from_block, to_block }) => {
                let result = (*from_block, *to_block);
                self.updates.pop_front();
                Some(result)
            }
            _ => None
        }
    }

    fn get_swap_event(&mut self) -> Option<(PoolId, u64, u64, u64, SwapEventData)> {
        match self.updates.front() {
            Some(PoolUpdate::SwapEvent { pool_id, block, tx_index, log_index, event }) => {
                let pool_id = *pool_id;
                let block = *block;
                let tx_index = *tx_index;
                let log_index = *log_index;
                let event = event.clone();
                self.updates.pop_front();
                Some((pool_id, block, tx_index, log_index, event))
            }
            _ => None
        }
    }

    fn get_liquidity_event(&mut self) -> Option<(PoolId, u64, u64, u64, ModifyLiquidityEventData)> {
        match self.updates.front() {
            Some(PoolUpdate::LiquidityEvent { pool_id, block, tx_index, log_index, event }) => {
                let pool_id = *pool_id;
                let block = *block;
                let tx_index = *tx_index;
                let log_index = *log_index;
                let event = event.clone();
                self.updates.pop_front();
                Some((pool_id, block, tx_index, log_index, event))
            }
            _ => None
        }
    }

    fn get_fee_update(&mut self) -> Option<(PoolId, u64, <T::FeeConfig as FeeConfig>::Update)> {
        match self.updates.front() {
            Some(PoolUpdate::FeeUpdate { pool_id, block, update }) => {
                let result = (*pool_id, *block, *update);
                self.updates.pop_front();
                Some(result)
            }
            _ => None
        }
    }

    fn get_slot0_update(&mut self) -> Option<(PoolId, Slot0Data)> {
        match self.updates.front() {
            Some(PoolUpdate::UpdatedSlot0 { pool_id, data }) => {
                let pool_id = *pool_id;
                let data = data.clone();
                self.updates.pop_front();
                Some((pool_id, data))
            }
            _ => None
        }
    }

    fn get_new_ticks(&mut self) -> Option<(PoolId, HashMap<i32, TickInfo>, HashMap<i16, U256>)> {
        match self.updates.front() {
            Some(PoolUpdate::NewTicks { pool_id, ticks, tick_bitmap }) => {
                let pool_id = *pool_id;
                let ticks = ticks.clone();
                let tick_bitmap = tick_bitmap.clone();
                self.updates.pop_front();
                Some((pool_id, ticks, tick_bitmap))
            }
            _ => None
        }
    }

    fn get_new_pool_state(&mut self) -> Option<(PoolId, BaselinePoolState<T>)> {
        match self.updates.front() {
            Some(PoolUpdate::NewPoolState { pool_id, state }) => {
                let pool_id = *pool_id;
                let state = state.clone();
                self.updates.pop_front();
                Some((pool_id, state))
            }
            _ => None
        }
    }

    fn get_chain_specific_update(&mut self) -> Option<(PoolId, T::PoolUpdate)> {
        match self.updates.front() {
            Some(PoolUpdate::ChainSpecific { pool_id, update }) => {
                let update = update.clone();
                let pool_id = *pool_id;
                self.updates.pop_front();
                Some((pool_id, update))
            }
            _ => None
        }
    }
}
