use alloy_primitives::{Address, B256};
use op_alloy_network::Optimism;

use crate::{BaselinePoolState, UpdatePool};

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
        hook_fee:             u32,
        tick_spacing:         i32,
        hook:                 Address,
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
        hook_fee: u32,
        hook: Address,
        block: u64
    ) -> Self {
        L2PoolUpdate::NewPool {
            pool_id,
            token0,
            token1,
            creator_tax_fee_e6,
            protocol_tax_fee_e6,
            creator_swap_fee_e6,
            hook_fee,
            hook,
            protocol_swap_fee_e6,
            tick_spacing,
            block
        }
    }
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
            L2PoolUpdate::NewPool { .. } => true
        }
    }
}
