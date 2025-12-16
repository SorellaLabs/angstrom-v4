use alloy_primitives::{Address, B256, I256};
use liquidity_base::BaselineLiquidity;
pub use pool_key::{PoolKey, PoolKeyWithFees};
use pool_swap::{PoolSwap, PoolSwapResult};
use serde::{Deserialize, Serialize};
use sqrt_pricex96::SqrtPriceX96;
pub use updates::UpdatePool;

use crate::fee_config::FeeConfig;
pub use crate::fee_config::{
    L1FeeConfiguration, L2_SWAP_MEV_TAX_FACTOR, L2_SWAP_TAXED_GAS, L2FeeConfiguration,
    calculate_l2_mev_tax
};
pub type PoolId = B256;

mod network;
pub mod updates;
pub use network::*;
mod address_book;
pub mod fee_config;
pub mod liquidity_base;
pub mod pool_key;
pub use address_book::*;
pub mod pool_registry;
pub mod pool_swap;
pub mod ray;
pub mod sqrt_pricex96;
pub mod tick_info;

//
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselinePoolState<T: V4Network> {
    liquidity:           BaselineLiquidity,
    block:               u64,
    fee_config:          T::FeeConfig,
    pub token0:          Address,
    pub token1:          Address,
    pub token0_decimals: u8,
    pub token1_decimals: u8
}

impl<T: V4Network> BaselinePoolState<T> {
    pub fn new(
        liquidity: BaselineLiquidity,
        block: u64,
        fee_config: T::FeeConfig,
        token0: Address,
        token1: Address,
        token0_decimals: u8,
        token1_decimals: u8
    ) -> Self {
        Self { liquidity, block, fee_config, token1, token0, token0_decimals, token1_decimals }
    }

    pub fn update_slot0(
        &mut self,
        start_tick: i32,
        start_sqrt_price: SqrtPriceX96,
        start_liquidity: u128
    ) {
        self.liquidity.start_tick = start_tick;
        self.liquidity.start_sqrt_price = start_sqrt_price;
        self.liquidity.start_liquidity = start_liquidity;
    }

    pub fn update_chain_specific(&mut self, update: T::PoolUpdate) {
        update.update_pool(self);
    }

    pub fn update_liquidity(&mut self, tick_lower: i32, tick_upper: i32, liquidity_delta: I256) {
        self.liquidity
            .update_liquidity_from_event(tick_lower, tick_upper, liquidity_delta);
    }

    pub fn block_number(&self) -> u64 {
        self.block
    }

    pub fn fees_mut(&mut self) -> &mut T::FeeConfig {
        &mut self.fee_config
    }

    pub fn fee(&self, bundle: bool) -> u32 {
        self.fee_config.fee(bundle)
    }

    pub fn bundle_fee(&self) -> Option<u32> {
        self.fee_config.bundle_fee()
    }

    pub fn swap_fee(&self) -> u32 {
        self.fee_config.swap_fee()
    }

    pub fn protocol_fee(&self) -> u32 {
        self.fee_config.protocol_fee()
    }

    pub fn fee_config(&self) -> &T::FeeConfig {
        &self.fee_config
    }

    pub fn current_tick(&self) -> i32 {
        self.liquidity.start_tick
    }

    pub fn current_liquidity(&self) -> u128 {
        self.liquidity.start_liquidity
    }

    pub fn current_price(&self) -> SqrtPriceX96 {
        self.liquidity.start_sqrt_price
    }

    pub fn tick_spacing(&self) -> i32 {
        self.liquidity.tick_spacing
    }

    pub fn noop(&self) -> PoolSwapResult<'_, T> {
        PoolSwapResult {
            fee_config:    self.fee_config.clone(),
            start_price:   self.liquidity.start_sqrt_price,
            start_tick:    self.liquidity.start_tick,
            end_price:     self.liquidity.start_sqrt_price,
            end_tick:      self.liquidity.start_tick,
            total_d_t0:    0,
            total_d_t1:    0,
            steps:         vec![],
            end_liquidity: self.liquidity.current(),
            is_bundle:     true
        }
    }

    pub fn swap_current_with_amount(
        &self,
        amount: I256,
        direction: bool,
        is_bundle: bool
    ) -> eyre::Result<PoolSwapResult<'_, T>> {
        let liq = self.liquidity.current();

        PoolSwap {
            liquidity: liq,
            target_amount: amount,
            target_price: None,
            direction,
            fee_config: self.fee_config.clone(),
            is_bundle,
            mev_tax_amount: None
        }
        .swap()
    }

    /// L2 swap with MEV tax applied to token0 (ETH) delta.
    /// Pass the priority fee (tx.gasprice - block.basefee) in wei to calculate
    /// the MEV tax.
    pub fn swap_current_with_amount_and_mev_tax(
        &self,
        amount: I256,
        direction: bool,
        is_bundle: bool,
        priority_fee_wei: Option<u128>
    ) -> eyre::Result<PoolSwapResult<'_, T>> {
        let liq = self.liquidity.current();
        let mev_tax_amount = priority_fee_wei.map(calculate_l2_mev_tax);

        PoolSwap {
            liquidity: liq,
            target_amount: amount,
            target_price: None,
            direction,
            fee_config: self.fee_config.clone(),
            is_bundle,
            mev_tax_amount
        }
        .swap()
    }

    pub fn swap_current_with_amount_and_limit(
        &self,
        amount: I256,
        direction: bool,
        is_bundle: bool,
        limit_price: SqrtPriceX96
    ) -> eyre::Result<PoolSwapResult<'_, T>> {
        let liq = self.liquidity.current();

        PoolSwap {
            liquidity: liq,
            target_amount: amount,
            target_price: Some(limit_price),
            direction,
            fee_config: self.fee_config.clone(),
            is_bundle,
            mev_tax_amount: None
        }
        .swap()
    }

    /// L2 swap with price limit and MEV tax applied to token0 (ETH) delta.
    /// Pass the priority fee (tx.gasprice - block.basefee) in wei to calculate
    /// the MEV tax.
    pub fn swap_current_with_amount_limit_and_mev_tax(
        &self,
        amount: I256,
        direction: bool,
        is_bundle: bool,
        limit_price: SqrtPriceX96,
        priority_fee_wei: Option<u128>
    ) -> eyre::Result<PoolSwapResult<'_, T>> {
        let liq = self.liquidity.current();
        let mev_tax_amount = priority_fee_wei.map(calculate_l2_mev_tax);

        PoolSwap {
            liquidity: liq,
            target_amount: amount,
            target_price: Some(limit_price),
            direction,
            fee_config: self.fee_config.clone(),
            is_bundle,
            mev_tax_amount
        }
        .swap()
    }

    /// Swap to current price is designed to represent all swap outcomes as an
    /// amount in swap. Because of this, this swap does two swaps to make
    /// sure the values always align perfectly.
    pub fn swap_current_to_price(
        &self,
        price_limit: SqrtPriceX96,
        is_bundle: bool
    ) -> eyre::Result<PoolSwapResult<'_, T>> {
        self.swap_current_to_price_with_mev_tax(price_limit, is_bundle, None)
    }

    /// L2 swap to price with MEV tax applied to token0 (ETH) delta.
    /// Pass the priority fee (tx.gasprice - block.basefee) in wei to calculate
    /// the MEV tax.
    pub fn swap_current_to_price_with_mev_tax(
        &self,
        price_limit: SqrtPriceX96,
        is_bundle: bool,
        priority_fee_wei: Option<u128>
    ) -> eyre::Result<PoolSwapResult<'_, T>> {
        let liq = self.liquidity.current();
        let direction = liq.current_sqrt_price >= price_limit;
        let mev_tax_amount = priority_fee_wei.map(calculate_l2_mev_tax);

        let price_swap: PoolSwapResult<'_, T> = PoolSwap {
            liquidity: liq,
            target_amount: I256::MAX,
            target_price: Some(price_limit),
            direction,
            fee_config: self.fee_config.clone(),
            is_bundle,
            mev_tax_amount: None // Don't apply MEV tax to price discovery swap
        }
        .swap()?;

        let amount_in = if direction { price_swap.total_d_t0 } else { price_swap.total_d_t1 };
        let amount = I256::unchecked_from(amount_in);

        let liq = self.liquidity.current();

        PoolSwap {
            liquidity: liq,
            target_amount: amount,
            target_price: None,
            direction,
            fee_config: self.fee_config.clone(),
            is_bundle,
            mev_tax_amount
        }
        .swap()
    }

    /// Angstrom operates everything on amount in, If we don't need this
    /// precision, then we can do raw.
    pub fn swap_current_to_price_raw(
        &self,
        price_limit: SqrtPriceX96,
        is_bundle: bool
    ) -> eyre::Result<PoolSwapResult<'_, T>> {
        let liq = self.liquidity.current();

        let direction = liq.current_sqrt_price >= price_limit;

        PoolSwap {
            liquidity: liq,
            target_amount: I256::MAX,
            target_price: Some(price_limit),
            direction,
            fee_config: self.fee_config.clone(),
            is_bundle,
            mev_tax_amount: None
        }
        .swap()
    }

    /// L2 raw swap to price with MEV tax applied to token0 (ETH) delta.
    /// Pass the priority fee (tx.gasprice - block.basefee) in wei to calculate
    /// the MEV tax.
    pub fn swap_current_to_price_raw_with_mev_tax(
        &self,
        price_limit: SqrtPriceX96,
        is_bundle: bool,
        priority_fee_wei: Option<u128>
    ) -> eyre::Result<PoolSwapResult<'_, T>> {
        let liq = self.liquidity.current();
        let mev_tax_amount = priority_fee_wei.map(calculate_l2_mev_tax);

        let direction = liq.current_sqrt_price >= price_limit;

        PoolSwap {
            liquidity: liq,
            target_amount: I256::MAX,
            target_price: Some(price_limit),
            direction,
            fee_config: self.fee_config.clone(),
            is_bundle,
            mev_tax_amount
        }
        .swap()
    }

    pub fn get_baseline_liquidity(&self) -> &BaselineLiquidity {
        &self.liquidity
    }

    pub fn get_baseline_liquidity_mut(&mut self) -> &mut BaselineLiquidity {
        &mut self.liquidity
    }
}
