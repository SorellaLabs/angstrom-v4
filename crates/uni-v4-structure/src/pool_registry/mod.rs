use std::{collections::HashMap, fmt::Debug};

use alloy_primitives::Address;

use crate::{PoolId, PoolKey};
pub mod l1;
pub mod l2;

pub trait PoolRegistry: Clone + Send + Sync + Unpin + Debug {
    type PoolIdSet: UniswapPoolIdSet;

    fn get(&self, pool_id: &PoolId) -> Option<&PoolKey>;

    fn pools(&self, hook: Option<Address>) -> HashMap<PoolId, PoolKey>;

    fn remove(&mut self, pool_id: &PoolId);

    // /// returns the uniswap pool ids
    // fn private_keys(&self, hook: Option<Address>) -> impl Iterator<Item = PoolId>
    // + '_;

    // /// returns the angstrom pool ids (if applicable)
    // fn public_keys(&self, hook: Option<Address>) -> impl Iterator<Item = PoolId>
    // + '_;

    fn all_angstrom_pool_ids(&self) -> impl Iterator<Item = PoolId> + '_;

    fn angstrom_pool_id_from_uniswap_pool_id(&self, pool_id: PoolId) -> Option<PoolId>;

    fn all_uniswap_pool_ids(&self) -> impl Iterator<Item = PoolId> + '_;

    fn add_new_pool(&mut self, pool_key: PoolKey);

    fn add_new_pools(&mut self, pool_keys: impl IntoIterator<Item = PoolKey>) {
        pool_keys
            .into_iter()
            .for_each(|pool_key| self.add_new_pool(pool_key));
    }

    /// Get pool key by token pair (searches all pools with these tokens)
    /// Returns all pools that match the token pair, regardless of fee tier
    fn get_pools_by_token_pair(
        &self,
        token0: Address,
        token1: Address,
        hook: Option<Address>
    ) -> Vec<&PoolKey>;

    /// Get pool ID by token pair and fee
    /// Returns the pool ID if a pool exists with the given tokens and fee
    fn get_pool_id_by_tokens_and_fee(
        &self,
        token0: Address,
        token1: Address,
        fee: u32,
        hook: Option<Address>
    ) -> Option<PoolId>;

    fn make_pool_id_set(&self, pool_id: PoolId) -> Option<Self::PoolIdSet>;
}

pub trait UniswapPoolIdSet: Copy + Clone + Send + Sync + Unpin + Debug {
    fn uniswap_pool_id(&self) -> PoolId;
}
