use std::{cmp::Ordering, collections::HashMap};

use alloy_network::Ethereum;
use alloy_primitives::{Address, B256, I256, U160, U256};
use serde::{Deserialize, Serialize};

use crate::{BaselinePoolState, V4Network, fee_config::FeeConfig, tick_info::TickInfo};

type PoolId = B256;

pub trait UpdatePool<T: V4Network>: Clone + Send + Sync + Unpin {
    /// updates the pool from the event
    fn update_pool(&self, pool: &mut BaselinePoolState<T>);

    /// whether or not to notify wakers
    fn should_notify_waiters(&self) -> bool;

    /// conditional check against the current block
    fn valid_current_block(&self, current_block_number: u64) -> bool;

    /// whether event changed pool state
    fn is_pool_affected(&self) -> bool;

    /// whether event initialization-related updates
    fn is_initialization_event(&self) -> bool;
}

/// Different types of pool updates
#[derive(Debug, Clone)]
pub enum PoolUpdate<T: V4Network> {
    /// New block notification
    NewBlock(u64),

    /// Swap event occurred
    SwapEvent {
        pool_id:   PoolId,
        block:     u64,
        tx_index:  u64,
        log_index: u64,
        event:     SwapEventData
    },
    /// Liquidity event occurred
    LiquidityEvent {
        pool_id:   PoolId,
        block:     u64,
        tx_index:  u64,
        log_index: u64,
        event:     ModifyLiquidityEventData
    },
    /// Reorg detected
    Reorg {
        from_block: u64,
        to_block:   u64
    },

    // From factory
    /// New ticks loaded for a pool
    NewTicks {
        pool_id:     PoolId,
        ticks:       HashMap<i32, TickInfo>,
        tick_bitmap: HashMap<i16, U256>
    },
    /// New pool with full state from factory
    NewPoolState {
        pool_id: PoolId,
        state:   BaselinePoolState<T>
    },

    /// Fee update event. the pool_id here is the uniswap pool_id
    FeeUpdate {
        pool_id: PoolId,
        block:   u64,
        update:  <T::FeeConfig as FeeConfig>::Update
    },

    /// Updated slot0 data after reorg
    UpdatedSlot0 {
        pool_id: B256,
        data:    Slot0Data
    },

    ChainSpecific {
        pool_id: PoolId,
        update:  T::PoolUpdate
    }
}

impl<T: V4Network> PoolUpdate<T> {
    pub fn sort(&self, b: &Self) -> Ordering {
        let (this_tx_index, this_log_index) = match self {
            PoolUpdate::SwapEvent { tx_index, log_index, .. } => (*tx_index, *log_index),
            PoolUpdate::LiquidityEvent { tx_index, log_index, .. } => (*tx_index, *log_index),
            _ => (u64::MAX, u64::MAX)
        };

        let (other_tx_index, other_log_index) = match b {
            PoolUpdate::SwapEvent { tx_index, log_index, .. } => (*tx_index, *log_index),
            PoolUpdate::LiquidityEvent { tx_index, log_index, .. } => (*tx_index, *log_index),
            _ => (u64::MAX, u64::MAX)
        };

        this_tx_index
            .cmp(&other_tx_index)
            .then_with(|| this_log_index.cmp(&other_log_index))
    }

    // Helper constructors
    pub fn from_swap(
        pool_id: PoolId,
        block: u64,
        tx_index: u64,
        log_index: u64,
        event: SwapEventData
    ) -> Self {
        PoolUpdate::SwapEvent { pool_id, block, tx_index, log_index, event }
    }

    pub fn from_liquidity(
        pool_id: PoolId,
        block: u64,
        tx_index: u64,
        log_index: u64,
        event: ModifyLiquidityEventData
    ) -> Self {
        PoolUpdate::LiquidityEvent { pool_id, block, tx_index, log_index, event }
    }

    pub fn from_fee_update(
        pool_id: PoolId,
        block: u64,
        update: <T::FeeConfig as FeeConfig>::Update
    ) -> Self {
        PoolUpdate::FeeUpdate { pool_id, block, update }
    }
}

/// Swap event data
#[derive(Debug, Clone)]
pub struct SwapEventData {
    pub sender:         Address,
    pub amount0:        i128,
    pub amount1:        i128,
    pub sqrt_price_x96: U160,
    pub liquidity:      u128,
    pub tick:           i32,
    pub fee:            u32
}

/// Modify liquidity event data
#[derive(Debug, Clone)]
pub struct ModifyLiquidityEventData {
    pub sender:          Address,
    pub tick_lower:      i32,
    pub tick_upper:      i32,
    pub liquidity_delta: I256,
    pub salt:            [u8; 32]
}

/// Current slot0 data for a pool
#[derive(Debug, Clone)]
pub struct Slot0Data {
    pub sqrt_price_x96: U160,
    pub tick:           i32,
    pub liquidity:      u128
}

/// Different types of pool updates
#[derive(Debug, Clone)]
pub enum L1PoolUpdate {
    NewPool {
        pool_id:      B256,
        token0:       Address,
        token1:       Address,
        bundle_fee:   u32,
        swap_fee:     u32,
        protocol_fee: u32,
        tick_spacing: i32,
        block:        u64
    },

    // From slot0 stream
    /// Real-time slot0 update
    Slot0Update(Slot0Update),

    /// Pool removed via controller
    PoolRemoved { pool_id: B256, block: u64 }
}

/// Slot0 update from real-time feed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct Slot0Update {
    /// there will be 120 updates per block or per 100ms
    pub seq_id:           u16,
    /// in case of block lag on node
    pub current_block:    u64,
    pub angstrom_pool_id: B256,
    pub uni_pool_id:      B256,

    pub sqrt_price_x96: U160,
    pub liquidity:      u128,
    pub tick:           i32
}

impl UpdatePool<Ethereum> for L1PoolUpdate {
    fn should_notify_waiters(&self) -> bool {
        matches!(self, Self::Slot0Update(_))
    }

    fn valid_current_block(&self, current_block_number: u64) -> bool {
        match self {
            L1PoolUpdate::Slot0Update(update) => update.current_block == current_block_number,
            _ => true
        }
    }

    fn update_pool(&self, pool: &mut BaselinePoolState<Ethereum>) {
        if let L1PoolUpdate::Slot0Update(update) = self {
            pool.update_slot0(update.tick, update.sqrt_price_x96.into(), update.liquidity);
        }
    }

    fn is_pool_affected(&self) -> bool {
        match self {
            L1PoolUpdate::NewPool { .. }
            | L1PoolUpdate::Slot0Update(_)
            | L1PoolUpdate::PoolRemoved { .. } => true
        }
    }

    fn is_initialization_event(&self) -> bool {
        match self {
            L1PoolUpdate::NewPool { .. } | L1PoolUpdate::PoolRemoved { .. } => true,
            L1PoolUpdate::Slot0Update(_) => false
        }
    }
}
