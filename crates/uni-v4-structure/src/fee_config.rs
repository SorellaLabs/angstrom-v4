use std::{fmt::Debug, hash::Hash};

use serde::{Deserialize, Serialize};

/// L2 MEV tax constants from AngstromL2.sol
/// The `SWAP_TAXED_GAS` is the abstract estimated gas cost for a swap.
pub const L2_SWAP_TAXED_GAS: u128 = 100_000;
/// MEV tax charged is `priority_fee * SWAP_MEV_TAX_FACTOR` meaning the tax rate
/// is `SWAP_MEV_TAX_FACTOR / (SWAP_MEV_TAX_FACTOR + 1)`
pub const L2_SWAP_MEV_TAX_FACTOR: u128 = 99;

/// Fee configuration for different pool modes
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct L1FeeConfiguration {
    pub bundle_fee:   u32, // Stored fee for bundle mode
    pub swap_fee:     u32, // Applied during swaps in unlocked mode
    pub protocol_fee: u32  // Applied after swaps in unlocked mode (basis points in 1e6)
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct L2FeeConfiguration {
    pub is_initialized:         bool,
    pub creator_tax_fee_e6:     u32,
    pub protocol_tax_fee_e6:    u32,
    pub creator_swap_fee_e6:    u32,
    pub protocol_swap_fee_e6:   u32,
    pub priority_fee_tax_floor: u128,
    pub jit_tax_enabled:        bool,
    pub withdraw_only:          bool
}

pub trait FeeConfig:
    Debug + Clone + Copy + PartialEq + Eq + Hash + Ord + PartialOrd + Send + Sync + Unpin
{
    type Update: Debug + Clone + Copy + Send + Sync + Unpin;

    /// Returns the swap fee applied during the swap (in compute_swap_step).
    /// - L1: LP fee charged during swap
    /// - L2: 0 (no LP fee during swap, all fees applied after)
    fn swap_fee(&self) -> u32;
    /// Returns the protocol fee applied after the swap on the output token.
    /// - L1: protocol_fee applied after swap
    /// - L2: (creator_swap_fee_e6 + protocol_swap_fee_e6) applied after swap
    fn protocol_fee(&self) -> u32;

    fn bundle_fee(&self) -> Option<u32>;

    /// Returns the total fee for a swap.
    /// - L1 bundle mode: uses bundle_fee
    /// - L1 unlocked mode: swap_fee + protocol_fee
    /// - L2: swap_fee (0) + protocol_fee (creator + protocol swap fees)
    fn fee(&self, bundle: bool) -> u32;

    fn priority_fee_tax_floor(&self) -> u128 {
        0
    }

    fn update_fees(&mut self, update: Self::Update);

    /// Calculate MEV tax given a priority fee in wei.
    /// Default returns 0 (no MEV tax, used by L1).
    /// L2 implements: SWAP_MEV_TAX_FACTOR * SWAP_TAXED_GAS * (priority_fee -
    /// floor)
    fn mev_tax(&self, _priority_fee_wei: u128) -> u128 {
        0
    }
}

impl FeeConfig for L1FeeConfiguration {
    type Update = L1FeeUpdate;

    fn protocol_fee(&self) -> u32 {
        self.protocol_fee
    }

    fn swap_fee(&self) -> u32 {
        self.swap_fee
    }

    fn bundle_fee(&self) -> Option<u32> {
        Some(self.bundle_fee)
    }

    fn fee(&self, bundle: bool) -> u32 {
        if bundle {
            self.bundle_fee()
                .expect("bundle fee must have a value if bundle is set to true")
        } else {
            self.swap_fee() + self.protocol_fee()
        }
    }

    fn update_fees(&mut self, update: Self::Update) {
        self.bundle_fee = update.bundle_fee;
        self.protocol_fee = update.protocol_fee;
        self.swap_fee = update.swap_fee;
    }
}

impl FeeConfig for L2FeeConfiguration {
    type Update = L2FeeUpdate;

    fn protocol_fee(&self) -> u32 {
        self.creator_swap_fee_e6 + self.protocol_swap_fee_e6
    }

    fn swap_fee(&self) -> u32 {
        0
    }

    fn bundle_fee(&self) -> Option<u32> {
        None
    }

    fn fee(&self, _: bool) -> u32 {
        self.swap_fee() + self.protocol_fee()
    }

    fn priority_fee_tax_floor(&self) -> u128 {
        self.priority_fee_tax_floor
    }

    fn update_fees(&mut self, update: Self::Update) {
        if let Some(fee) = update.protocol_tax_fee_e6 {
            self.protocol_tax_fee_e6 = fee;
        }
        if let Some(fee) = update.protocol_swap_fee_e6 {
            self.protocol_swap_fee_e6 = fee;
        }
        if let Some(floor) = update.priority_fee_tax_floor {
            self.priority_fee_tax_floor = floor;
        }
        if let Some(enabled) = update.jit_tax_enabled {
            self.jit_tax_enabled = enabled;
        }
        if let Some(wo) = update.withdraw_only {
            self.withdraw_only = wo;
        }
    }

    fn mev_tax(&self, priority_fee_wei: u128) -> u128 {
        if priority_fee_wei <= self.priority_fee_tax_floor {
            return 0;
        }
        L2_SWAP_MEV_TAX_FACTOR
            * L2_SWAP_TAXED_GAS
            * (priority_fee_wei - self.priority_fee_tax_floor)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct L1FeeUpdate {
    pub bundle_fee:   u32,
    pub swap_fee:     u32,
    pub protocol_fee: u32
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct L2FeeUpdate {
    pub protocol_tax_fee_e6:    Option<u32>,
    pub protocol_swap_fee_e6:   Option<u32>,
    pub priority_fee_tax_floor: Option<u128>,
    pub jit_tax_enabled:        Option<bool>,
    pub withdraw_only:          Option<bool>
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l2_fee_config(floor: u128) -> L2FeeConfiguration {
        L2FeeConfiguration {
            is_initialized:         true,
            creator_tax_fee_e6:     1000,
            protocol_tax_fee_e6:    2000,
            creator_swap_fee_e6:    3000,
            protocol_swap_fee_e6:   4000,
            priority_fee_tax_floor: floor,
            jit_tax_enabled:        false,
            withdraw_only:          false
        }
    }

    #[test]
    fn l1_mev_tax_always_zero() {
        let cfg = L1FeeConfiguration { bundle_fee: 100, swap_fee: 200, protocol_fee: 300 };
        assert_eq!(cfg.mev_tax(0), 0);
        assert_eq!(cfg.mev_tax(1_000_000_000), 0);
        assert_eq!(cfg.mev_tax(u128::MAX), 0);
    }

    #[test]
    fn l2_mev_tax_zero_floor() {
        let cfg = l2_fee_config(0);
        // 99 * 100_000 * 1 = 9_900_000
        assert_eq!(cfg.mev_tax(1), 9_900_000);
        // 99 * 100_000 * 1_000_000_000 (1 gwei) = 9_900_000_000_000_000
        assert_eq!(cfg.mev_tax(1_000_000_000), 9_900_000_000_000_000);
    }

    #[test]
    fn l2_mev_tax_zero_when_at_floor() {
        let cfg = l2_fee_config(500);
        assert_eq!(cfg.mev_tax(500), 0);
    }

    #[test]
    fn l2_mev_tax_zero_when_below_floor() {
        let cfg = l2_fee_config(500);
        assert_eq!(cfg.mev_tax(0), 0);
        assert_eq!(cfg.mev_tax(499), 0);
    }

    #[test]
    fn l2_mev_tax_subtracts_floor() {
        let cfg = l2_fee_config(100);
        // priority_fee=150, effective=50
        // 99 * 100_000 * 50 = 495_000_000
        assert_eq!(cfg.mev_tax(150), 99 * 100_000 * 50);
    }

    #[test]
    fn l2_update_fees_floor_some() {
        let mut cfg = l2_fee_config(0);
        cfg.update_fees(L2FeeUpdate {
            protocol_tax_fee_e6:    None,
            protocol_swap_fee_e6:   None,
            priority_fee_tax_floor: Some(42),
            jit_tax_enabled:        None,
            withdraw_only:          None
        });
        assert_eq!(cfg.priority_fee_tax_floor, 42);
    }

    #[test]
    fn l2_update_fees_floor_none_unchanged() {
        let mut cfg = l2_fee_config(100);
        cfg.update_fees(L2FeeUpdate {
            protocol_tax_fee_e6:    Some(9999),
            protocol_swap_fee_e6:   None,
            priority_fee_tax_floor: None,
            jit_tax_enabled:        None,
            withdraw_only:          None
        });
        assert_eq!(cfg.priority_fee_tax_floor, 100);
        assert_eq!(cfg.protocol_tax_fee_e6, 9999);
    }

    #[test]
    fn l2_update_fees_mixed() {
        let mut cfg = l2_fee_config(0);
        cfg.update_fees(L2FeeUpdate {
            protocol_tax_fee_e6:    Some(111),
            protocol_swap_fee_e6:   Some(222),
            priority_fee_tax_floor: Some(333),
            jit_tax_enabled:        Some(true),
            withdraw_only:          Some(true)
        });
        assert_eq!(cfg.protocol_tax_fee_e6, 111);
        assert_eq!(cfg.protocol_swap_fee_e6, 222);
        assert_eq!(cfg.priority_fee_tax_floor, 333);
        assert!(cfg.jit_tax_enabled);
        assert!(cfg.withdraw_only);
    }
}
