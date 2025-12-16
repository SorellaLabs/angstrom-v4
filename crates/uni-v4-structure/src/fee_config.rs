use std::fmt::Debug;

use serde::{Deserialize, Serialize};

/// L2 MEV tax constants from AngstromL2.sol
/// The `SWAP_TAXED_GAS` is the abstract estimated gas cost for a swap.
pub const L2_SWAP_TAXED_GAS: u128 = 100_000;
/// MEV tax charged is `priority_fee * SWAP_MEV_TAX_FACTOR` meaning the tax rate
/// is `SWAP_MEV_TAX_FACTOR / (SWAP_MEV_TAX_FACTOR + 1)`
pub const L2_SWAP_MEV_TAX_FACTOR: u128 = 49;

/// Calculate the L2 MEV tax amount given a priority fee (in wei).
/// This matches `getSwapTaxAmount` in AngstromL2.sol:
/// `SWAP_MEV_TAX_FACTOR * SWAP_TAXED_GAS * priorityFee`
pub fn calculate_l2_mev_tax(priority_fee_wei: u128) -> u128 {
    L2_SWAP_MEV_TAX_FACTOR * L2_SWAP_TAXED_GAS * priority_fee_wei
}

/// Fee configuration for different pool modes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct L1FeeConfiguration {
    pub bundle_fee:   u32, // Stored fee for bundle mode
    pub swap_fee:     u32, // Applied during swaps in unlocked mode
    pub protocol_fee: u32  // Applied after swaps in unlocked mode (basis points in 1e6)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct L2FeeConfiguration {
    pub is_initialized:       bool,
    pub creator_tax_fee_e6:   u32,
    pub protocol_tax_fee_e6:  u32,
    pub creator_swap_fee_e6:  u32,
    pub protocol_swap_fee_e6: u32
}

// #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// pub enum FeeConfiguration {
//     L1(L1FeeConfiguration),
//     L2(L2FeeConfiguration)
// }

// impl FeeConfiguration {
//     /// Create a new L1 fee configuration
//     pub fn new_l1(bundle_fee: u32, swap_fee: u32, protocol_fee: u32) -> Self
// {         FeeConfiguration::L1(L1FeeConfiguration { bundle_fee, swap_fee,
// protocol_fee })     }

//     /// Create a new L2 fee configuration
//     pub fn new_l2(
//         is_initialized: bool,
//         creator_tax_fee_e6: u32,
//         protocol_tax_fee_e6: u32,
//         creator_swap_fee_e6: u32,
//         protocol_swap_fee_e6: u32
//     ) -> Self {
//         FeeConfiguration::L2(L2FeeConfiguration {
//             is_initialized,
//             creator_tax_fee_e6,
//             protocol_tax_fee_e6,
//             creator_swap_fee_e6,
//             protocol_swap_fee_e6
//         })
//     }

//     /// Returns the swap fee applied during the swap (in compute_swap_step).
//     /// - L1: LP fee charged during swap
//     /// - L2: 0 (no LP fee during swap, all fees applied after)
//     pub fn swap_fee(&self) -> u32 {
//         match self {
//             FeeConfiguration::L1(cfg) => cfg.swap_fee,
//             // L2 returns 0 for swap fee in beforeSwap - all fees are applied
// in afterSwap             FeeConfiguration::L2(_) => 0
//         }
//     }

//     /// Returns the protocol fee applied after the swap on the output token.
//     /// - L1: protocol_fee applied after swap
//     /// - L2: (creator_swap_fee_e6 + protocol_swap_fee_e6) applied after swap
//     pub fn protocol_fee(&self) -> u32 {
//         match self {
//             FeeConfiguration::L1(cfg) => cfg.protocol_fee,
//             // L2 applies all swap fees (creator + protocol) in afterSwap
//             FeeConfiguration::L2(cfg) => cfg.creator_swap_fee_e6 +
// cfg.protocol_swap_fee_e6         }
//     }

//     pub fn update_fees(
//         &mut self,
//         bundle_fee: Option<u32>,
//         swap_fee: Option<u32>,
//         protocol_fee: Option<u32>
//     ) {
//         match self {
//             FeeConfiguration::L1(cfg) => {
//                 if let Some(fee) = bundle_fee {
//                     cfg.bundle_fee = fee;
//                 }
//                 if let Some(fee) = swap_fee {
//                     cfg.swap_fee = fee;
//                 }
//                 if let Some(fee) = protocol_fee {
//                     cfg.protocol_fee = fee;
//                 }
//             }
//             FeeConfiguration::L2(cfg) => {
//                 if let Some(fee) = protocol_fee {
//                     cfg.protocol_swap_fee_e6 = fee;
//                 }
//             }
//         }
//     }

//     pub fn update_l2_fees(
//         &mut self,
//         creator_tax_fee_e6: Option<u32>,
//         protocol_tax_fee_e6: Option<u32>,
//         creator_swap_fee_e6: Option<u32>,
//         protocol_swap_fee_e6: Option<u32>
//     ) {
//         match self {
//             FeeConfiguration::L1(_) => {
//                 panic!("update_l2_fees called on L1 configuration")
//             }
//             FeeConfiguration::L2(cfg) => {
//                 if let Some(fee) = creator_tax_fee_e6 {
//                     cfg.creator_tax_fee_e6 = fee;
//                 }
//                 if let Some(fee) = protocol_tax_fee_e6 {
//                     cfg.protocol_tax_fee_e6 = fee;
//                 }
//                 if let Some(fee) = creator_swap_fee_e6 {
//                     cfg.creator_swap_fee_e6 = fee;
//                 }
//                 if let Some(fee) = protocol_swap_fee_e6 {
//                     cfg.protocol_swap_fee_e6 = fee;
//                 }
//             }
//         }
//     }

//     /// Returns true if this is an L1 fee configuration
//     pub fn is_l1(&self) -> bool {
//         matches!(self, FeeConfiguration::L1(_))
//     }

//     /// Returns true if this is an L2 fee configuration
//     pub fn is_l2(&self) -> bool {
//         matches!(self, FeeConfiguration::L2(_))
//     }
// }

pub trait FeeConfig: Debug + Clone + Send + Sync + Unpin {
    type Update: Debug + Clone + Copy + Send + Sync;

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

    fn update_fees(&mut self, update: Self::Update);
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

    fn update_fees(&mut self, update: Self::Update) {}
}

#[derive(Debug, Clone, Copy)]
pub struct L1FeeUpdate {
    pub bundle_fee:   u32,
    pub swap_fee:     u32,
    pub protocol_fee: u32
}

#[derive(Debug, Clone, Copy)]
pub struct L2FeeUpdate {}
