use std::collections::HashMap;

use alloy_primitives::U256;
use uni_v4_structure::{
    BaselinePoolState, PoolId,
    fee_config::FeeConfig,
    tick_info::TickInfo,
    updates::{ModifyLiquidityEventData, PoolUpdate, Slot0Data, SwapEventData}
};

use crate::V4Network;

/// Trait for delivering pool updates from various data sources
///
/// Implement this trait on your data delivery struct to provide pool updates.
/// Each method returns the specific data needed to construct a `PoolUpdate`
/// variant. Return `None` if no update of that type is available.
pub trait PoolUpdateDelivery<T: V4Network>: Send + Sync {
    /// Get notification of a new block
    fn get_new_block(&mut self) -> Option<u64>;

    /// Get notification of a chain reorganization
    /// Returns: (from_block, to_block)
    fn get_reorg(&mut self) -> Option<(u64, u64)>;

    // /// Get a new pool creation event
    // /// Returns: (pool_id, token0, token1, bundle_fee, swap_fee, protocol_fee,
    // /// tick_spacing, block)
    // fn get_new_pool(&mut self) -> Option<(PoolId, Address, Address, u32, u32, u32, i32, u64)>;

    // /// Get a pool removal event
    // /// Returns: (pool_id, block)
    // fn get_pool_removal(&mut self) -> Option<(PoolId, u64)>;

    /// Get a swap event
    /// Returns: (pool_id, block, tx_index, log_index, event_data)
    fn get_swap_event(&mut self) -> Option<(PoolId, u64, u64, u64, SwapEventData)>;

    /// Get a liquidity modification event
    /// Returns: (pool_id, block, tx_index, log_index, event_data)
    fn get_liquidity_event(&mut self) -> Option<(PoolId, u64, u64, u64, ModifyLiquidityEventData)>;

    /// Get a fee update event
    /// Returns: (pool_id, block, bundle_fee, swap_fee, protocol_fee)
    fn get_fee_update(&mut self) -> Option<(PoolId, u64, <T::FeeConfig as FeeConfig>::Update)>;

    /// Get a slot0 update (real-time price/liquidity/tick update)
    /// Returns: (pool_id, slot0_data)
    fn get_slot0_update(&mut self) -> Option<(PoolId, Slot0Data)>;

    /// Get new ticks loaded for a pool
    /// Returns: (pool_id, ticks, tick_bitmap)
    fn get_new_ticks(&mut self) -> Option<(PoolId, HashMap<i32, TickInfo>, HashMap<i16, U256>)>;

    /// Get a new pool with full state
    /// Returns: (pool_id, state)
    fn get_new_pool_state(&mut self) -> Option<(PoolId, BaselinePoolState<T>)>;

    // /// Get a slot0 streaming update
    // fn get_slot0_stream_update(&mut self) -> Option<Slot0Update>;

    /// Get chain specific update
    fn get_chain_specific_update(&mut self) -> Option<(PoolId, T::PoolUpdate)>;
}

/// Extension trait for PoolUpdateDelivery that provides a method to get the
/// next available update
pub trait PoolUpdateDeliveryExt<T: V4Network>: PoolUpdateDelivery<T> {
    /// Get the next available update of any type
    fn next_update(&mut self) -> Option<PoolUpdate<T>> {
        // Try each update type in priority order
        if let Some(block) = self.get_new_block() {
            return Some(PoolUpdate::NewBlock(block));
        }

        if let Some((from_block, to_block)) = self.get_reorg() {
            return Some(PoolUpdate::Reorg { from_block, to_block });
        }

        // if let Some((
        //     pool_id,
        //     token0,
        //     token1,
        //     bundle_fee,
        //     swap_fee,
        //     protocol_fee,
        //     tick_spacing,
        //     block
        // )) = self.get_new_pool()
        // {
        //     return Some(PoolUpdate::from_new_pool(
        //         pool_id,
        //         token0,
        //         token1,
        //         bundle_fee,
        //         swap_fee,
        //         protocol_fee,
        //         tick_spacing,
        //         block
        //     ));
        // }

        // if let Some((pool_id, block)) = self.get_pool_removal() {
        //     return Some(PoolUpdate::PoolRemoved { pool_id, block });
        // }

        if let Some((pool_id, block, tx_index, log_index, event)) = self.get_swap_event() {
            return Some(PoolUpdate::from_swap(pool_id, block, tx_index, log_index, event));
        }

        if let Some((pool_id, block, tx_index, log_index, event)) = self.get_liquidity_event() {
            return Some(PoolUpdate::from_liquidity(pool_id, block, tx_index, log_index, event));
        }

        if let Some((pool_id, block, update)) = self.get_fee_update() {
            return Some(PoolUpdate::from_fee_update(pool_id, block, update));
        }

        if let Some((pool_id, data)) = self.get_slot0_update() {
            return Some(PoolUpdate::UpdatedSlot0 { pool_id, data });
        }

        if let Some((pool_id, ticks, tick_bitmap)) = self.get_new_ticks() {
            return Some(PoolUpdate::NewTicks { pool_id, ticks, tick_bitmap });
        }

        if let Some((pool_id, state)) = self.get_new_pool_state() {
            return Some(PoolUpdate::NewPoolState { pool_id, state });
        }

        // if let Some(update) = self.get_slot0_stream_update() {
        //     return Some(PoolUpdate::Slot0Update(update));
        // }

        if let Some((pool_id, update)) = self.get_chain_specific_update() {
            return Some(PoolUpdate::ChainSpecific { pool_id, update });
        }

        None
    }
}

// Blanket implementation
impl<D: PoolUpdateDelivery<T>, T: V4Network> PoolUpdateDeliveryExt<T> for D {}
