use std::collections::{HashMap, HashSet};

use alloy_consensus::Transaction;
use alloy_eips::BlockId;
use alloy_network::{BlockResponse, Ethereum};
use alloy_primitives::{Address, aliases::I24};
use alloy_provider::Provider;
use alloy_rpc_types::Filter;
use alloy_sol_types::{SolCall, SolEvent};
use futures::StreamExt;
// pub use types::*;
use uni_v4_common::{PoolUpdate, V4Network};
use uni_v4_structure::{
    L1AddressBook, L1FeeConfiguration, PoolId, PoolKey, PoolKeyWithFees, fee_config::L1FeeUpdate,
    pool_registry::PoolRegistry, updates::l1::L1PoolUpdate
};

use crate::pool_providers::{
    ProviderChainUpdate,
    update_providers::{PoolUpdateError, PoolUpdateProvider}
};
mod types {
    alloy_sol_types::sol! {
        #[derive(Debug, PartialEq, Eq)]
        contract ControllerV1 {

            event PoolConfigured(
                address indexed asset0,
                address indexed asset1,
                uint16 tickSpacing,
                uint24 bundleFee,
                uint24 unlockedFee,
                uint24 protocolUnlockedFee
            );

            event PoolRemoved(
                address indexed asset0, address indexed asset1, int24 tickSpacing, uint24 feeInE6
            );

            struct PoolUpdate {
                address assetA;
                address assetB;
                uint24 bundleFee;
                uint24 unlockedFee;
                uint24 protocolUnlockedFee;
            }

            function batchUpdatePools(PoolUpdate[] calldata updates) external;
        }
    }
}

impl<P> ProviderChainUpdate<Ethereum> for PoolUpdateProvider<P, Ethereum>
where
    P: Provider<Ethereum>
{
    async fn fetch_chain_data(
        &mut self,
        from_block: u64,
        to_block: u64
    ) -> Result<Vec<PoolUpdate<Ethereum>>, PoolUpdateError> {
        let mut updates =
            self.process_controller_logs(self.fetch_controller_logs(from_block, to_block).await?);

        updates.extend(
            self.fetch_controller_batch_updates(from_block, to_block)
                .await?
        );

        Ok(updates)
    }
}

impl<P> PoolUpdateProvider<P, Ethereum>
where
    P: Provider<Ethereum> + 'static
{
    async fn fetch_controller_logs(
        &self,
        from_block: u64,
        to_block: u64
    ) -> Result<Vec<alloy_rpc_types::Log>, PoolUpdateError> {
        // Query controller events
        let controller_filter = Filter::new()
            .address(self.address_book().controller_v1)
            .from_block(from_block)
            .to_block(to_block);

        let controller_logs = self
            .provider
            .get_logs(&controller_filter)
            .await
            .map_err(|e| {
                PoolUpdateError::Provider(format!("Failed to get controller logs: {e}"))
            })?;

        Ok(controller_logs)
    }

    async fn fetch_controller_batch_updates(
        &self,
        from_block: u64,
        to_block: u64
    ) -> Result<Vec<PoolUpdate<Ethereum>>, PoolUpdateError> {
        let mut updates = Vec::new();
        // Process transactions to find batchUpdatePools calls
        // For single blocks, get the block directly. For ranges, iterate.
        if from_block == to_block {
            let block = self
                .provider
                .get_block(BlockId::Number(from_block.into()))
                .full()
                .await
                .map_err(|e| PoolUpdateError::Provider(format!("Failed to get block: {e}")))?
                .ok_or_else(|| PoolUpdateError::Provider("Block not found".to_string()))?;

            if let Some(transactions) = block.transactions().as_transactions() {
                for tx in transactions {
                    updates.extend(self.process_batch_update_pools(tx, from_block));
                }
            }
        } else {
            // For block ranges, iterate through each block
            for block_num in from_block..=to_block {
                let block = self
                    .provider
                    .get_block(BlockId::Number(block_num.into()))
                    .full()
                    .await
                    .map_err(|e| PoolUpdateError::Provider(format!("Failed to get block: {e}")))?;

                if let Some(block) = block
                    && let Some(transactions) = block.transactions().as_transactions()
                {
                    for tx in transactions {
                        updates.extend(self.process_batch_update_pools(tx, block_num));
                    }
                }
            }
        }
        Ok(updates)
    }

    /// Process controller event logs
    fn process_controller_logs(
        &mut self,
        logs: Vec<alloy_rpc_types::Log>
    ) -> Vec<PoolUpdate<Ethereum>> {
        let mut updates = Vec::new();

        for log in logs {
            let block_number = log.block_number.unwrap();

            if let Ok(event) = types::ControllerV1::PoolConfigured::decode_log(&log.inner) {
                let pool_key = PoolKey {
                    currency0:   event.asset0,
                    currency1:   event.asset1,
                    fee:         event.bundleFee,
                    tickSpacing: I24::unchecked_from(event.tickSpacing),
                    hooks:       self.address_book().angstrom
                };

                self.pool_registry.add_new_pool(pool_key);

                // Get the Uniswap pool ID from registry
                let angstrom_pool_id = PoolId::from(pool_key);
                let pool_id = self
                    .pool_registry
                    .private_key_from_public(&angstrom_pool_id)
                    .unwrap();

                updates.push(PoolUpdate::ChainSpecific {
                    pool_id,
                    update: L1PoolUpdate::NewPool {
                        pool_id,
                        token0: pool_key.currency0,
                        token1: pool_key.currency1,
                        bundle_fee: event.bundleFee.to(),
                        swap_fee: event.unlockedFee.to(),
                        protocol_fee: event.protocolUnlockedFee.to(),
                        tick_spacing: event.tickSpacing as i32,
                        block: block_number
                    }
                });
            }

            if let Ok(event) = types::ControllerV1::PoolRemoved::decode_log(&log.inner) {
                let pool_key = PoolKey {
                    currency0:   event.asset0,
                    currency1:   event.asset1,
                    fee:         event.feeInE6,
                    tickSpacing: event.tickSpacing,
                    hooks:       self.address_book().angstrom
                };

                // Get the Uniswap pool ID from registry
                let angstrom_pool_id = PoolId::from(pool_key);
                let pool_id = self
                    .pool_registry
                    .private_key_from_public(&angstrom_pool_id)
                    .unwrap();

                updates.push(PoolUpdate::ChainSpecific {
                    pool_id,
                    update: L1PoolUpdate::PoolRemoved { pool_id, block: block_number }
                });
            }
        }

        updates
    }

    /// Process batch update pools from transaction
    fn process_batch_update_pools(
        &self,
        tx: &alloy_rpc_types::Transaction,
        block_number: u64
    ) -> Vec<PoolUpdate<Ethereum>> {
        let mut updates = Vec::new();

        // Check if transaction is to the controller
        if tx.to() == Some(self.address_book().controller_v1) {
            // Try to decode as batchUpdatePools call
            if let Ok(call) = types::ControllerV1::batchUpdatePoolsCall::abi_decode(tx.input()) {
                for update in call.updates {
                    // Normalize asset order
                    let (_asset0, _asset1) = if update.assetB > update.assetA {
                        (update.assetA, update.assetB)
                    } else {
                        (update.assetB, update.assetA)
                    };
                    let pools = self.pool_registry.get_pools_by_token_pair(
                        update.assetA,
                        update.assetB,
                        Some(self.address_book().angstrom)
                    );

                    // Find the pool with matching fee (or just use the first one if no match)
                    let pool_key = pools
                        .iter()
                        .find(|pk| pk.fee.to::<u32>() == update.bundleFee.to::<u32>())
                        .or_else(|| pools.first())
                        .cloned()
                        .cloned();

                    if let Some(pool_key) = pool_key {
                        // Get the Uniswap pool ID from registry
                        let angstrom_pool_id = PoolId::from(pool_key);
                        let pool_id = self
                            .pool_registry
                            .private_key_from_public(&angstrom_pool_id)
                            .unwrap();

                        updates.push(PoolUpdate::FeeUpdate {
                            pool_id,
                            block: block_number,
                            update: L1FeeUpdate {
                                bundle_fee:   update.bundleFee.to(),
                                swap_fee:     update.unlockedFee.to(),
                                protocol_fee: update.protocolUnlockedFee.to()
                            }
                        });
                    }
                }
            }
        }

        updates
    }
}

pub async fn fetch_angstrom_pools<P>(
    mut deploy_block: u64,
    end_block: u64,
    angstrom_address: Address,
    controller_address: Address,
    db: &P
) -> Vec<PoolKeyWithFees<L1FeeConfiguration>>
where
    P: Provider<Ethereum>
{
    let mut filters = vec![];

    loop {
        let this_end_block = std::cmp::min(deploy_block + 99_999, end_block);

        if this_end_block == deploy_block {
            break;
        }

        tracing::info!(?deploy_block, ?this_end_block);
        let filter = Filter::new()
            .from_block(deploy_block as u64)
            .to_block(this_end_block as u64)
            .address(controller_address);

        filters.push(filter);

        deploy_block = std::cmp::min(end_block, this_end_block);
    }

    let logs = futures::stream::iter(filters)
        .map(|filter| async move {
            db.get_logs(&filter)
                .await
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .buffered(10)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    logs.into_iter()
        .fold(HashMap::new(), |mut set, log| {
            if let Ok(pool) =
                types::ControllerV1::PoolConfigured::decode_log(&log.clone().into_inner())
            {
                let pool_key_with_fees = PoolKeyWithFees {
                    pool_key: PoolKey {
                        currency0:   pool.asset0,
                        currency1:   pool.asset1,
                        fee:         pool.bundleFee,
                        tickSpacing: I24::unchecked_from(pool.tickSpacing),
                        hooks:       angstrom_address
                    },
                    fee_cfg:  L1FeeConfiguration {
                        bundle_fee:   pool.bundleFee.to(),
                        swap_fee:     pool.unlockedFee.to(),
                        protocol_fee: pool.protocolUnlockedFee.to()
                    }
                };

                let mut raw = pool_key_with_fees.pool_key.clone();
                raw.fee = Default::default();

                set.insert(raw, pool_key_with_fees);
                return set;
            }

            if let Ok(pool) =
                types::ControllerV1::PoolRemoved::decode_log(&log.clone().into_inner())
            {
                let remove_key = PoolKey {
                    currency0:   pool.asset0,
                    currency1:   pool.asset1,
                    fee:         Default::default(),
                    tickSpacing: pool.tickSpacing,
                    hooks:       angstrom_address
                };
                set.remove(&remove_key);
                return set;
            }
            set
        })
        .into_iter()
        .map(|(_, key)| key)
        .collect::<Vec<_>>()
}
