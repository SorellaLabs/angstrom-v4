use std::ops::Neg;

use alloy_primitives::{I256, U256};
// use itertools::Itertools;
use uniswap_v3_math::tick_math::{MAX_SQRT_RATIO, MIN_SQRT_RATIO};

use super::liquidity_base::LiquidityAtPoint;
use crate::{V4Network, fee_config::FeeConfig, ray::Ray, sqrt_pricex96::SqrtPriceX96};

const U256_1: U256 = U256::from_limbs([1, 0, 0, 0]);

#[derive(Debug, Clone)]
pub struct PoolSwap<'a, T: V4Network> {
    pub(super) liquidity:      LiquidityAtPoint<'a>,
    /// swap to sqrt price limit
    pub(super) target_price:   Option<SqrtPriceX96>,
    /// if its negative, it is an exact out.
    pub(super) target_amount:  I256,
    /// zfo = true
    pub(super) direction:      bool,
    // the fee configuration of the pool.
    pub(super) fee_config:     T::FeeConfig,
    pub(super) is_bundle:      bool,
    /// L2 MEV tax amount in wei (only applicable for L2 pools).
    /// Calculated via `fee_config.mev_tax(priority_fee)` which accounts for
    /// the priority fee tax floor.
    pub(super) mev_tax_amount: Option<u128>
}

impl<'a, T: V4Network> PoolSwap<'a, T> {
    pub fn swap(mut self) -> eyre::Result<PoolSwapResult<'a, T>> {
        // We want to ensure that we set the right limits and are swapping the correct
        // way.

        if self.direction
            && self
                .target_price
                .as_ref()
                .map(|target_price| target_price > &self.liquidity.current_sqrt_price)
                .unwrap_or_default()
        {
            return Err(eyre::eyre!("direction and sqrt_price diverge"));
        }

        let range_start = self.liquidity.current_sqrt_price;
        let range_start_tick = self.liquidity.current_tick;

        let exact_input = self.target_amount.is_positive();
        let sqrt_price_limit_x96 = self.target_price.map(|p| p.into()).unwrap_or_else(|| {
            if self.direction { MIN_SQRT_RATIO + U256_1 } else { MAX_SQRT_RATIO - U256_1 }
        });

        // L2 BeforeSwapDelta: deduct protocol fee + MEV tax from input BEFORE AMM.
        // This mirrors AngstromL2.sol's beforeSwap which returns a BeforeSwapDelta
        // that reduces amountSpecified before the pool swap runs.
        //
        // For exact input:
        //   - swap fee (protocol_fee) is taken from the input token amount
        //   - MEV tax is taken from ETH (token0): from input if ETH is input, from
        //     output if ETH is output
        //   - AMM runs with reduced input and LP fee = 0
        let protocol_fee_rate = self.fee_config.protocol_fee();
        let mev_tax = self.mev_tax_amount.unwrap_or(0);
        let ether_is_input = self.direction; // zeroForOne = selling ETH

        // Calculate how much of the input to deduct before AMM
        let (before_swap_input_deduction, before_swap_output_deduction) =
            if !self.is_bundle && exact_input && protocol_fee_rate > 0 {
                let input_amount = self.target_amount.unsigned_abs();

                // MEV tax is on ETH. If ETH is the input token, subtract it first
                // (matches Solidity: `if (etherIsInput) inputAmount -= swapTax`)
                let taxable_input = if ether_is_input {
                    input_amount.saturating_sub(U256::from(mev_tax))
                } else {
                    input_amount
                };

                let fee_amount =
                    taxable_input * U256::from(protocol_fee_rate) / U256::from(1_000_000u32);

                if ether_is_input {
                    // ETH→CBBTC: both mev_tax and fee deducted from input (specified)
                    (mev_tax + fee_amount.saturating_to::<u128>(), 0u128)
                } else {
                    // CBBTC→ETH: fee from input (specified), mev_tax from output (unspecified/ETH)
                    (fee_amount.saturating_to::<u128>(), mev_tax)
                }
            } else if !self.is_bundle && exact_input && mev_tax > 0 {
                // No protocol fee but MEV tax applies
                if ether_is_input { (mev_tax, 0u128) } else { (0u128, mev_tax) }
            } else {
                (0u128, 0u128)
            };

        // Reduce input amount by beforeSwap deduction (fees taken before AMM)
        let mut amount_remaining = if before_swap_input_deduction > 0 && exact_input {
            self.target_amount
                .saturating_sub(I256::from_raw(U256::from(before_swap_input_deduction)))
        } else {
            self.target_amount
        };
        let mut sqrt_price_x96: U256 = self.liquidity.current_sqrt_price.into();

        let mut steps = Vec::new();

        while amount_remaining != I256::ZERO && sqrt_price_x96 != sqrt_price_limit_x96 {
            let sqrt_price_start_x_96 = sqrt_price_x96;

            let (next_tick, liquidity, init) = self
                .liquidity
                .get_to_next_initialized_tick_within_one_word(self.direction)?;

            let sqrt_price_next_x96 =
                uniswap_v3_math::tick_math::get_sqrt_ratio_at_tick(next_tick)?;

            let target_sqrt_ratio = if (self.direction
                && sqrt_price_next_x96 < sqrt_price_limit_x96)
                || (!self.direction && sqrt_price_next_x96 > sqrt_price_limit_x96)
            {
                sqrt_price_limit_x96
            } else {
                sqrt_price_next_x96
            };

            // Use 0 fee for bundle mode, swap_fee for unlocked mode
            let swap_fee = if self.is_bundle { 0 } else { self.fee_config.swap_fee() };

            let (new_sqrt_price_x_96, amount_in, amount_out, fee_amount) =
                uniswap_v3_math::swap_math::compute_swap_step(
                    sqrt_price_x96,
                    target_sqrt_ratio,
                    liquidity,
                    amount_remaining,
                    swap_fee
                )?;

            sqrt_price_x96 = new_sqrt_price_x_96;
            if exact_input {
                // swap amount is positive so we sub
                amount_remaining = amount_remaining.saturating_sub(I256::from_raw(amount_in));
                amount_remaining = amount_remaining.saturating_sub(I256::from_raw(fee_amount));
            } else {
                // we add as is neg
                amount_remaining = amount_remaining.saturating_add(I256::from_raw(amount_out));
            }

            let (d_t0, d_t1) = if self.direction {
                // zero-for-one swap: token0 in, token1 out
                // fee is always on the input (token0) side
                ((amount_in + fee_amount).to(), amount_out.to())
            } else {
                // one-for-zero swap: token1 in, token0 out
                // fee is always on the input (token1) side
                (amount_out.to(), (amount_in + fee_amount).to())
            };

            self.liquidity.move_to_next_tick(
                sqrt_price_x96,
                self.direction,
                sqrt_price_x96 == sqrt_price_next_x96,
                sqrt_price_x96 != sqrt_price_start_x_96
            )?;

            steps.push(PoolSwapStep { end_tick: next_tick, init, liquidity, d_t0, d_t1 });
        }

        // the final sqrt price
        self.liquidity.set_sqrt_price(sqrt_price_x96);

        let (total_d_t0, total_d_t1) = steps.iter().fold((0u128, 0u128), |(mut t0, mut t1), x| {
            t0 += x.d_t0;
            t1 += x.d_t1;
            (t0, t1)
        });

        // Apply beforeSwap deductions to the final deltas.
        // The AMM ran on a reduced input, so AMM deltas reflect the post-fee amounts.
        // We need to add back the fees to the input side (caller pays full amount)
        // and subtract MEV tax from output if applicable.
        let (final_d_t0, final_d_t1) =
            if before_swap_input_deduction > 0 || before_swap_output_deduction > 0 {
                if self.direction {
                    // zeroForOne: token0 is input, token1 is output
                    // Add input deductions (mev_tax + fee) back to token0 input
                    let adj_t0 = total_d_t0.saturating_add(before_swap_input_deduction);
                    (adj_t0, total_d_t1)
                } else {
                    // oneForZero: token1 is input, token0 is output
                    // Add input deductions (fee) back to token1 input
                    let adj_t1 = total_d_t1.saturating_add(before_swap_input_deduction);
                    // Subtract output deductions (mev_tax) from token0 output
                    let adj_t0 = total_d_t0.saturating_sub(before_swap_output_deduction);
                    (adj_t0, adj_t1)
                }
            } else {
                (total_d_t0, total_d_t1)
            };

        Ok(PoolSwapResult {
            fee_config: self.fee_config,
            start_price: range_start,
            start_tick: range_start_tick,
            end_price: self.liquidity.current_sqrt_price,
            end_tick: self.liquidity.current_tick,
            total_d_t0: final_d_t0,
            total_d_t1: final_d_t1,
            steps,
            end_liquidity: self.liquidity,
            is_bundle: self.is_bundle
        })
    }
}

#[derive(Debug, Clone)]
pub struct PoolSwapResult<'a, T: V4Network> {
    pub fee_config:    T::FeeConfig,
    pub start_price:   SqrtPriceX96,
    pub start_tick:    i32,
    pub end_price:     SqrtPriceX96,
    pub end_tick:      i32,
    pub total_d_t0:    u128,
    pub total_d_t1:    u128,
    pub steps:         Vec<PoolSwapStep>,
    pub end_liquidity: LiquidityAtPoint<'a>,
    pub is_bundle:     bool
}

impl<'a, T: V4Network> PoolSwapResult<'a, T> {
    /// initialize a swap from the end of this swap into a new swap.
    pub fn swap_to_amount(
        &'a self,
        amount: I256,
        direction: bool
    ) -> eyre::Result<PoolSwapResult<'a, T>> {
        self.swap_to_amount_with_mev_tax(amount, direction, None)
    }

    /// Initialize a swap from the end of this swap with MEV tax applied.
    /// Pass the priority fee (tx.gasprice - block.basefee) in wei to calculate
    /// the MEV tax.
    pub fn swap_to_amount_with_mev_tax(
        &'a self,
        amount: I256,
        direction: bool,
        priority_fee_wei: Option<u128>
    ) -> eyre::Result<PoolSwapResult<'a, T>> {
        let mev_tax_amount = priority_fee_wei
            .map(|fee| self.fee_config.mev_tax(fee))
            .filter(|&tax| tax > 0);
        PoolSwap {
            liquidity: self.end_liquidity.clone(),
            target_price: None,
            direction,
            target_amount: amount,
            fee_config: self.fee_config,
            is_bundle: self.is_bundle,
            mev_tax_amount
        }
        .swap()
    }

    pub fn swap_to_price(
        &'a self,
        price_limit: SqrtPriceX96
    ) -> eyre::Result<PoolSwapResult<'a, T>> {
        self.swap_to_price_with_mev_tax(price_limit, None)
    }

    /// Swap to price with MEV tax applied.
    /// Pass the priority fee (tx.gasprice - block.basefee) in wei to calculate
    /// the MEV tax.
    pub fn swap_to_price_with_mev_tax(
        &'a self,
        price_limit: SqrtPriceX96,
        priority_fee_wei: Option<u128>
    ) -> eyre::Result<PoolSwapResult<'a, T>> {
        let direction = self.end_price >= price_limit;

        let price_swap: PoolSwapResult<'_, T> = PoolSwap {
            liquidity: self.end_liquidity.clone(),
            target_price: Some(price_limit),
            direction,
            target_amount: I256::MAX,
            fee_config: self.fee_config,
            is_bundle: self.is_bundle,
            mev_tax_amount: None // Don't apply MEV tax to price discovery swap
        }
        .swap()?;

        let amount_in = if direction { price_swap.total_d_t0 } else { price_swap.total_d_t1 };
        let amount = I256::unchecked_from(amount_in);

        self.swap_to_amount_with_mev_tax(amount, direction, priority_fee_wei)
    }

    pub fn was_empty_swap(&self) -> bool {
        self.total_d_t0 == 0 || self.total_d_t1 == 0
    }

    /// Returns the amount of T0 exchanged over this swap with a sign attached,
    /// negative if performing this swap consumes T0 (T0 is the input quantity
    /// for the described swap) and positive if performing this swap provides T0
    /// (T0 is the output quantity for the described swap)
    pub fn t0_signed(&self) -> I256 {
        let val = I256::unchecked_from(self.total_d_t0);
        if self.zero_for_one() { val.neg() } else { val }
    }

    /// Returns the amount of T1 exchanged over this swap with a sign attached,
    /// negative if performing this swap consumes T1 (T1 is the input quantity
    /// for the described swap) and positive if performing this swap provides T1
    /// (T1 is the output quantity for the described swap)
    pub fn t1_signed(&self) -> I256 {
        let val = I256::unchecked_from(self.total_d_t1);
        if self.zero_for_one() { val } else { val.neg() }
    }

    /// Returns a boolean indicating whether this PoolPriceVec is
    /// `zero_for_one`.  This will be true if the AMM is buying T0 and the AMM
    /// price is decreasing, false if the AMM is selling T0 and the AMM price is
    /// increasing.
    pub fn zero_for_one(&self) -> bool {
        self.start_price > self.end_price
    }

    pub fn input(&self) -> u128 {
        if self.zero_for_one() { self.total_d_t0 } else { self.total_d_t1 }
    }

    pub fn output(&self) -> u128 {
        if self.zero_for_one() { self.total_d_t1 } else { self.total_d_t0 }
    }
}

/// the step of swapping across this pool
#[derive(Clone, Debug)]
pub struct PoolSwapStep {
    pub end_tick:  i32,
    pub init:      bool,
    pub liquidity: u128,
    pub d_t0:      u128,
    pub d_t1:      u128
}

impl PoolSwapStep {
    pub fn avg_price(&self) -> Option<Ray> {
        if self.empty() {
            None
        } else {
            Some(Ray::calc_price(U256::from(self.d_t0), U256::from(self.d_t1)))
        }
    }

    pub fn empty(&self) -> bool {
        self.d_t0 == 0 || self.d_t1 == 0
    }
}
