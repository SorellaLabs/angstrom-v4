use alloy_network::Ethereum;
use alloy_primitives::{Address, B256, U160};
use serde::{Deserialize, Serialize};

use crate::{BaselinePoolState, UpdatePool};

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

impl L1PoolUpdate {
    pub fn from_new_pool(
        pool_id: B256,
        token0: Address,
        token1: Address,
        bundle_fee: u32,
        swap_fee: u32,
        protocol_fee: u32,
        tick_spacing: i32,
        block: u64
    ) -> Self {
        L1PoolUpdate::NewPool {
            pool_id,
            token0,
            token1,
            bundle_fee,
            swap_fee,
            protocol_fee,
            tick_spacing,
            block
        }
    }
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
