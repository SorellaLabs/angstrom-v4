use std::collections::HashMap;

use alloy_primitives::Address;

use crate::{
    PoolId, PoolKey,
    pool_registry::{PoolRegistry, UniswapPoolIdSet}
};

#[derive(Debug, Default, Clone)]
pub struct L2PoolRegistry {
    pools: HashMap<PoolId, PoolKey>
}

impl PoolRegistry for L2PoolRegistry {
    type PoolIdSet = PoolId;

    fn get(&self, pool_id: &PoolId) -> Option<&PoolKey> {
        self.pools.get(pool_id)
    }

    fn pools(&self, hook: Option<Address>) -> HashMap<PoolId, PoolKey> {
        if let Some(addr) = hook {
            self.pools
                .iter()
                .filter_map(|(id, key)| (key.hooks == addr).then_some((*id, key.clone())))
                .collect()
        } else {
            self.pools.clone()
        }
    }

    fn add_new_pool(&mut self, pool_key: PoolKey) {
        self.pools.insert(pool_key.into(), pool_key);
    }

    fn remove(&mut self, pool_id: &PoolId) {
        self.pools.remove(pool_id);
    }

    fn get_pools_by_token_pair(
        &self,
        token0: Address,
        token1: Address,
        hook: Option<Address>
    ) -> Vec<&PoolKey> {
        let (normalized_token0, normalized_token1) =
            if token0 < token1 { (token0, token1) } else { (token1, token0) };

        self.pools
            .values()
            .filter(|pool_key| {
                pool_key.currency0 == normalized_token0
                    && pool_key.currency1 == normalized_token1
                    && (hook.is_none() || hook == Some(pool_key.hooks))
            })
            .collect()
    }

    fn get_pool_id_by_tokens_and_fee(
        &self,
        token0: Address,
        token1: Address,
        fee: u32,
        hook: Option<Address>
    ) -> Option<PoolId> {
        let (normalized_token0, normalized_token1) =
            if token0 < token1 { (token0, token1) } else { (token1, token0) };

        self.pools
            .iter()
            .find(|(_, pool_key)| {
                pool_key.currency0 == normalized_token0
                    && pool_key.currency1 == normalized_token1
                    && pool_key.fee.to::<u32>() == fee
                    && (hook.is_none() || hook == Some(pool_key.hooks))
            })
            .map(|(pool_id, _)| *pool_id)
    }

    fn make_pool_id_set(&self, pool_id: PoolId) -> Option<Self::PoolIdSet> {
        self.pools.contains_key(&pool_id).then_some(pool_id)
    }

    fn all_angstrom_pool_ids(&self) -> impl Iterator<Item = PoolId> + '_ {
        Vec::new().into_iter()
    }

    fn all_uniswap_pool_ids(&self) -> impl Iterator<Item = PoolId> + '_ {
        self.pools.keys().cloned()
    }

    fn angstrom_pool_id_from_uniswap_pool_id(&self, pool_id: PoolId) -> Option<PoolId> {
        None
    }
}

impl UniswapPoolIdSet for PoolId {
    fn uniswap_pool_id(&self) -> PoolId {
        *self
    }
}
