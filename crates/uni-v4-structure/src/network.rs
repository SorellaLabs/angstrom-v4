use alloy_network::{Ethereum, Network};

use crate::{
    L1AddressBook, L1FeeConfiguration, UpdatePool,
    fee_config::FeeConfig,
    pool_registry::{L1PoolRegistry, PoolRegistry},
    pool_updates::L1PoolUpdate
};

pub trait V4Network: Network + Send + Sync + Unpin {
    type PoolUpdate: UpdatePool<Self>;
    type FeeConfig: FeeConfig;
    type AddressBook: Copy + Send + Sync + Unpin;
    type PoolRegistry: PoolRegistry;
}

impl V4Network for Ethereum {
    type AddressBook = L1AddressBook;
    type FeeConfig = L1FeeConfiguration;
    type PoolRegistry = L1PoolRegistry;
    type PoolUpdate = L1PoolUpdate;
}
