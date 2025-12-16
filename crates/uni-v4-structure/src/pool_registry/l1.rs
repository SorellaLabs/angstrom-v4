use std::collections::HashMap;

use alloy_primitives::{Address, aliases::U24};

use crate::{
    PoolId, PoolKey,
    pool_registry::{PoolRegistry, UniswapPoolIdSet}
};

#[derive(Debug, Clone)]
pub struct L1PoolRegistry {
    angstrom_address:  Address,
    uni_pools:         HashMap<PoolId, PoolKey>,
    angstrom_registry: AngstromRegistry
}

impl L1PoolRegistry {
    pub fn new(angstrom_address: Address) -> Self {
        Self {
            angstrom_address,
            uni_pools: Default::default(),
            angstrom_registry: Default::default()
        }
    }

    pub fn private_keys(&self) -> impl Iterator<Item = PoolId> + '_ {
        self.angstrom_registry.conversion_map.values().copied()
    }

    pub fn public_keys(&self) -> impl Iterator<Item = PoolId> + '_ {
        self.angstrom_registry.conversion_map.keys().copied()
    }

    pub fn public_key_from_private(&self, pk: &PoolId) -> Option<PoolId> {
        self.angstrom_registry
            .reverse_conversion_map
            .get(pk)
            .copied()
    }

    pub fn private_key_from_public(&self, pk: &PoolId) -> Option<PoolId> {
        self.angstrom_registry.conversion_map.get(pk).copied()
    }
}

impl PoolRegistry for L1PoolRegistry {
    type PoolIdSet = AngstromPoolIdPair;

    fn get(&self, pool_id: &PoolId) -> Option<&PoolKey> {
        let pool_key = self.uni_pools.get(pool_id);
        if pool_key.is_some() {
            pool_key
        } else {
            self.angstrom_registry.pools.get(pool_id)
        }
    }

    fn pools(&self, hook: Option<Address>) -> HashMap<PoolId, PoolKey> {
        if let Some(addr) = hook {
            self.uni_pools
                .iter()
                .filter_map(|(id, key)| (key.hooks == addr).then_some((*id, *key)))
                .collect()
        } else {
            self.uni_pools.clone()
        }
    }

    fn remove(&mut self, pool_id: &PoolId) {
        if let Some(id_set) = self.make_pool_id_set(*pool_id) {
            self.uni_pools.remove(&id_set.uniswap_id);
            self.angstrom_registry
                .reverse_conversion_map
                .remove(&id_set.uniswap_id);
            self.angstrom_registry
                .conversion_map
                .remove(&id_set.angstrom_id);
            self.angstrom_registry.pools.remove(&id_set.angstrom_id);
        }
    }

    fn add_new_pool(&mut self, mut pool_key: PoolKey) {
        if pool_key.hooks == self.angstrom_address {
            self.angstrom_registry.add_key(pool_key);
            pool_key.fee = U24::from(0x800000);
        }

        self.uni_pools.insert(pool_key.into(), pool_key);
    }

    fn get_pools_by_token_pair(
        &self,
        token0: Address,
        token1: Address,
        hook: Option<Address>
    ) -> Vec<&PoolKey> {
        let (normalized_token0, normalized_token1) =
            if token0 < token1 { (token0, token1) } else { (token1, token0) };

        self.uni_pools
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

        self.uni_pools
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
        if self.uni_pools.contains_key(&pool_id) {
            let angstrom_id = self
                .angstrom_registry
                .reverse_conversion_map
                .get(&pool_id)?;
            Some(Self::PoolIdSet { angstrom_id: *angstrom_id, uniswap_id: pool_id })
        } else if self.angstrom_registry.pools.contains_key(&pool_id) {
            let uniswap_id = self.angstrom_registry.conversion_map.get(&pool_id)?;

            Some(Self::PoolIdSet { angstrom_id: pool_id, uniswap_id: *uniswap_id })
        } else {
            None
        }
    }

    fn all_angstrom_pool_ids(&self) -> impl Iterator<Item = PoolId> + '_ {
        self.angstrom_registry.pools.keys().cloned()
    }

    fn all_uniswap_pool_ids(&self) -> impl Iterator<Item = PoolId> + '_ {
        self.uni_pools.keys().cloned()
    }

    fn angstrom_pool_id_from_uniswap_pool_id(&self, pool_id: PoolId) -> Option<PoolId> {
        self.angstrom_registry
            .reverse_conversion_map
            .get(&pool_id)
            .copied()
    }
}

#[derive(Debug, Default, Clone)]
struct AngstromRegistry {
    pools:                  HashMap<PoolId, PoolKey>,
    conversion_map:         HashMap<PoolId, PoolId>,
    reverse_conversion_map: HashMap<PoolId, PoolId>
}

impl AngstromRegistry {
    fn add_key(&mut self, mut pool_key: PoolKey) {
        self.pools.insert(pool_key.into(), pool_key);
        let copyed_pub: PoolId = pool_key.into();

        pool_key.fee = U24::from(0x800000);
        let priv_key = PoolId::from(pool_key);
        self.conversion_map.insert(copyed_pub, priv_key);
        self.reverse_conversion_map.insert(priv_key, copyed_pub);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AngstromPoolIdPair {
    /// (prev. public_key)
    pub angstrom_id: PoolId,
    /// (prev. private_key)
    pub uniswap_id:  PoolId
}

impl AngstromPoolIdPair {
    pub fn new(angstrom_id: PoolId, uniswap_id: PoolId) -> Self {
        Self { angstrom_id, uniswap_id }
    }
}

impl From<PoolKey> for AngstromPoolIdPair {
    fn from(mut pool_key: PoolKey) -> Self {
        let angstrom_id: PoolId = pool_key.into();
        pool_key.fee = U24::from(0x800000);
        let uniswap_id = PoolId::from(pool_key);

        Self { angstrom_id, uniswap_id }
    }
}

impl UniswapPoolIdSet for AngstromPoolIdPair {
    fn uniswap_pool_id(&self) -> PoolId {
        self.uniswap_id
    }
}
