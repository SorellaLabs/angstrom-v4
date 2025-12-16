use alloy_primitives::{Address, B256, U160};
use op_alloy_network::Optimism;
use serde::{Deserialize, Serialize};

use crate::{BaselinePoolState, UpdatePool};

/// Different types of pool updates
#[derive(Debug, Clone)]
pub enum L2PoolUpdate {
    NewPool {
        pool_id:              B256,
        token0:               Address,
        token1:               Address,
        creator_tax_fee_e6:   u32,
        protocol_tax_fee_e6:  u32,
        creator_swap_fee_e6:  u32,
        protocol_swap_fee_e6: u32,
        tick_spacing:         i32,
        block:                u64
    }
}

impl L2PoolUpdate {
    pub fn from_new_pool(
        pool_id: B256,
        token0: Address,
        token1: Address,
        creator_tax_fee_e6: u32,
        protocol_tax_fee_e6: u32,
        creator_swap_fee_e6: u32,
        protocol_swap_fee_e6: u32,
        tick_spacing: i32,
        block: u64
    ) -> Self {
        L2PoolUpdate::NewPool {
            pool_id,
            token0,
            token1,
            creator_tax_fee_e6,
            protocol_tax_fee_e6,
            creator_swap_fee_e6,
            protocol_swap_fee_e6,
            tick_spacing,
            block
        }
    }
}

/// Current slot0 data for a pool
#[derive(Debug, Clone)]
pub struct Slot0Data {
    pub sqrt_price_x96: U160,
    pub tick:           i32,
    pub liquidity:      u128
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

impl UpdatePool<Optimism> for L2PoolUpdate {
    fn should_notify_waiters(&self) -> bool {
        false
    }

    fn valid_current_block(&self, _: u64) -> bool {
        true
    }

    fn update_pool(&self, _: &mut BaselinePoolState<Optimism>) {}

    fn is_pool_affected(&self) -> bool {
        match self {
            L2PoolUpdate::NewPool { .. } => true
        }
    }

    fn is_initialization_event(&self) -> bool {
        match self {
            L2PoolUpdate::NewPool { .. } => true,
            _ => false
        }
    }
}
