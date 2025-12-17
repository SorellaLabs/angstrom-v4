pub mod pools;

pub use uni_v4_structure::V4Network;

pub mod pool_update;
pub mod traits;

// Re-export commonly used types
pub use pools::{PoolError, SwapSimulationError, UniswapPools};
pub use traits::{PoolUpdateDelivery, PoolUpdateDeliveryExt};
pub use uni_v4_structure::updates::{ModifyLiquidityEventData, PoolUpdate, SwapEventData};

/// Configuration for what types of pool updates should be streamed
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    /// Stream all updates (default behavior)
    #[default]
    Full,
    /// Only stream initialization updates: new pools, fee updates, and slot0
    /// updates
    InitializationOnly
}
