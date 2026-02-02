use op_alloy_network::Optimism;

use crate::{L2FeeConfiguration, V4Network, l2::pool_registry::L2PoolRegistry};
mod address_book;
pub use address_book::*;
pub mod pool_registry;
pub mod pool_updates;
use pool_updates::L2PoolUpdate;

impl V4Network for Optimism {
    type AddressBook = L2AddressBook;
    type FeeConfig = L2FeeConfiguration;
    type PoolRegistry = L2PoolRegistry;
    type PoolUpdate = L2PoolUpdate;
}
