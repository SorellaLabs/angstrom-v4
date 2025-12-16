use alloy_network::{Ethereum, Network};
use op_alloy_network::Optimism;

use crate::{
    L1AddressBook, L1FeeConfiguration, L2AddressBook, L2FeeConfiguration, UpdatePool,
    fee_config::FeeConfig,
    pool_registry::UniswapPoolRegistry,
    updates::{l1::L1PoolUpdate, l2::L2PoolUpdate}
};

pub trait V4Network: Network + Send + Sync + Unpin {
    type PoolUpdate: UpdatePool<Self>;
    type FeeConfig: FeeConfig;
    type AddressBook: Copy + Send + Sync + Unpin;
    type PoolRegistry: Clone + Send + Sync + Unpin;
}

impl V4Network for Ethereum {
    type AddressBook = L1AddressBook;
    type FeeConfig = L1FeeConfiguration;
    type PoolRegistry = UniswapPoolRegistry;
    type PoolUpdate = L1PoolUpdate;
}

impl V4Network for Optimism {
    type AddressBook = L2AddressBook;
    type FeeConfig = L2FeeConfiguration;
    type PoolRegistry = UniswapPoolRegistry;
    type PoolUpdate = L2PoolUpdate;
}
