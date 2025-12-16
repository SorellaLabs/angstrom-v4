use std::collections::HashMap;

use alloy_primitives::{B256, FixedBytes};
use thiserror::Error;
use uni_v4_structure::{BaselinePoolState, fee_config::FeeConfig, updates::PoolUpdate};
use uniswap_v3_math::error::UniswapV3MathError;

use crate::{
    V4Network,
    traits::{PoolUpdateDelivery, PoolUpdateDeliveryExt}
};

/// Single-threaded version of UniswapPools without locking
pub struct UniswapPools<T: V4Network> {
    pools:        HashMap<PoolId, BaselinePoolState<T>>,
    // what block these are up to date for.
    block_number: u64
}

impl<T: V4Network> UniswapPools<T> {
    pub fn new(pools: HashMap<PoolId, BaselinePoolState<T>>, block_number: u64) -> Self {
        Self { pools, block_number }
    }

    pub fn get_block(&self) -> u64 {
        self.block_number
    }

    pub fn get_pool(&self, pool_id: &PoolId) -> Option<&BaselinePoolState<T>> {
        self.pools.get(pool_id)
    }

    pub fn get_pool_mut(&mut self, pool_id: &PoolId) -> Option<&mut BaselinePoolState<T>> {
        self.pools.get_mut(pool_id)
    }

    pub fn get_pools(&self) -> &HashMap<PoolId, BaselinePoolState<T>> {
        &self.pools
    }

    pub fn get_pools_mut(&mut self) -> &mut HashMap<PoolId, BaselinePoolState<T>> {
        &mut self.pools
    }

    pub fn insert_pool(&mut self, pool_id: PoolId, pool: BaselinePoolState<T>) {
        self.pools.insert(pool_id, pool);
    }

    pub fn remove_pool(&mut self, pool_id: &PoolId) -> Option<BaselinePoolState<T>> {
        self.pools.remove(pool_id)
    }

    pub fn update_pools(&mut self, mut updates: Vec<PoolUpdate<T>>) {
        if updates.is_empty() {
            return
        }

        let mut new_block_number = 0;
        // we sort ascending
        updates.sort_by(|a, b| a.sort(b));

        for update in updates {
            match update {
                PoolUpdate::NewBlock(block_number) => {
                    new_block_number = block_number;
                }
                PoolUpdate::Reorg { to_block, .. } => {
                    new_block_number = to_block;
                }
                PoolUpdate::SwapEvent { pool_id, event, .. } => {
                    let Some(pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };

                    // update slot0 values
                    pool.update_slot0(event.tick, event.sqrt_price_x96.into(), event.liquidity);
                }
                PoolUpdate::LiquidityEvent { pool_id, event, .. } => {
                    let Some(pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };

                    pool.update_liquidity(
                        event.tick_lower,
                        event.tick_upper,
                        event.liquidity_delta
                    );
                }
                PoolUpdate::FeeUpdate { pool_id, update, .. } => {
                    let Some(pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };
                    let fees = pool.fees_mut();
                    fees.update_fees(update);
                }
                // PoolUpdate::UpdatedSlot0 { pool_id, data } => {
                //     let Some(pool) = self.pools.get_mut(&pool_id) else {
                //         continue;
                //     };

                //     pool.update_slot0(data.tick, data.sqrt_price_x96.into(), data.liquidity);
                // }
                PoolUpdate::ChainSpecific { pool_id, update } => {
                    let Some(pool) = self.pools.get_mut(&pool_id) else {
                        continue;
                    };

                    pool.update_chain_specific(update);
                }
                _ => {}
            }
        }

        tracing::debug!("processed block: {}", new_block_number);

        assert!(
            new_block_number != 0,
            "Got a update but no block info with update. Should never happen"
        );

        self.block_number = new_block_number;
    }

    /// Update pools using a PoolUpdateDelivery source
    /// Processes all available updates from the source
    pub fn update_from_source<D: PoolUpdateDelivery<T>>(&mut self, source: &mut D) {
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
    pub fn update_single_from_source<D: PoolUpdateDelivery<T>>(&mut self, source: &mut D) -> bool {
        if let Some(update) = source.next_update() {
            self.update_pools(vec![update]);
            true
        } else {
            false
        }
    }
}

/// Pool identifier type
pub type PoolId = FixedBytes<32>;

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
