use std::collections::HashMap;

use alloy_primitives::{
    Address,
    aliases::{I24, U24}
};
use alloy_provider::Provider;
use alloy_rpc_types::Filter;
use alloy_sol_types::SolEvent;
use futures::StreamExt;
use itertools::Itertools;
use op_alloy_network::Optimism;
pub use types::*;
use uni_v4_common::PoolUpdate;
use uni_v4_structure::{
    L2AddressBook, L2FeeConfiguration, PoolId, PoolKey, PoolKeyWithFees, fee_config::L2FeeUpdate,
    pool_registry::PoolRegistry, updates::l2::L2PoolUpdate
};

use crate::pool_providers::{
    ProviderChainUpdate,
    update_providers::{PoolUpdateError, PoolUpdateProvider}
};

mod types {
    alloy_sol_types::sol! {
        #[derive(Debug, PartialEq, Eq)]
        contract AngstromL2Factory {
            type Currency is address;
            type IHooks is address;

            struct PoolKey {
                /// @notice The lower currency of the pool, sorted numerically
                Currency currency0;
                /// @notice The higher currency of the pool, sorted numerically
                Currency currency1;
                /// @notice The pool LP fee, capped at 1_000_000. If the highest bit is 1, the pool has a dynamic fee and must be exactly equal to 0x800000
                uint24 fee;
                /// @notice Ticks that involve positions must be a multiple of tick spacing
                int24 tickSpacing;
                /// @notice The hooks of the pool
                IHooks hooks;
            }

            event PoolCreated(
                address hook,
                PoolKey key,
                uint24 creatorSwapFeeE6,
                uint24 creatorTaxFeeE6,
                uint24 protocolSwapFeeE6,
                uint24 protocolTaxFeeE6
            );

            event ProtocolSwapFeeUpdated(address indexed hook, PoolKey key, uint256 newFeeE6);
            event ProtocolTaxFeeUpdated(address indexed hook, PoolKey key, uint256 newFeeE6);
        }
    }

    impl From<AngstromL2Factory::PoolKey> for super::PoolKey {
        fn from(value: AngstromL2Factory::PoolKey) -> Self {
            Self {
                currency0:   value.currency0,
                currency1:   value.currency1,
                fee:         value.fee,
                tickSpacing: value.tickSpacing,
                hooks:       value.hooks
            }
        }
    }
}

impl<P> ProviderChainUpdate<Optimism> for PoolUpdateProvider<P, Optimism>
where
    P: Provider<Optimism>
{
    async fn fetch_chain_data(
        &mut self,
        from_block: u64,
        to_block: u64
    ) -> Result<Vec<PoolUpdate<Optimism>>, PoolUpdateError> {
        let updates =
            self.process_l2_factory_logs(self.fetch_l2_factory_logs(from_block, to_block).await?);

        Ok(updates)
    }
}

impl<P> PoolUpdateProvider<P, Optimism>
where
    P: Provider<Optimism> + 'static
{
    async fn fetch_l2_factory_logs(
        &self,
        from_block: u64,
        to_block: u64
    ) -> Result<Vec<alloy_rpc_types::Log>, PoolUpdateError> {
        // Query l2 factory events
        let l2_factory_filter = Filter::new()
            .address(self.address_book().angstrom_v2_factory)
            .from_block(from_block)
            .to_block(to_block);

        let l2_factory_logs = self
            .provider
            .get_logs(&l2_factory_filter)
            .await
            .map_err(|e| {
                PoolUpdateError::Provider(format!("Failed to get l2 factory logs: {e}"))
            })?;

        Ok(l2_factory_logs)
    }

    /// Process L2 factory event logs
    fn process_l2_factory_logs(
        &mut self,
        logs: Vec<alloy_rpc_types::Log>
    ) -> Vec<PoolUpdate<Optimism>> {
        let mut updates = Vec::new();

        for log in logs {
            let block_number = log.block_number.unwrap();

            if let Ok(event) = AngstromL2Factory::PoolCreated::decode_log(&log.inner) {
                let pool_key = event.key.clone().into();

                self.pool_registry.add_new_pool(pool_key);

                let pool_id = PoolId::from(pool_key);

                updates.push(PoolUpdate::ChainSpecific {
                    pool_id,
                    update: L2PoolUpdate::NewPool {
                        pool_id,
                        token0: pool_key.currency0,
                        token1: pool_key.currency1,
                        hook: event.hook,
                        hook_fee: pool_key.fee.to(),
                        tick_spacing: pool_key.tickSpacing.as_i32(),
                        block: block_number,
                        creator_tax_fee_e6: event.creatorTaxFeeE6.to(),
                        protocol_tax_fee_e6: event.protocolTaxFeeE6.to(),
                        creator_swap_fee_e6: event.creatorSwapFeeE6.to(),
                        protocol_swap_fee_e6: event.protocolSwapFeeE6.to()
                    }
                });
            } else if let Ok(event) =
                AngstromL2Factory::ProtocolSwapFeeUpdated::decode_log(&log.inner)
            {
                let pool_id = PoolId::from(PoolKey::from(event.key.clone()));

                updates.push(PoolUpdate::FeeUpdate {
                    pool_id,
                    block: block_number,
                    update: L2FeeUpdate {
                        protocol_tax_fee_e6:  None,
                        protocol_swap_fee_e6: Some(event.data.newFeeE6.to())
                    }
                })
            } else if let Ok(event) =
                AngstromL2Factory::ProtocolTaxFeeUpdated::decode_log(&log.inner)
            {
                let pool_id = PoolId::from(PoolKey::from(event.key.clone()));

                updates.push(PoolUpdate::FeeUpdate {
                    pool_id,
                    block: block_number,
                    update: L2FeeUpdate {
                        protocol_tax_fee_e6:  Some(event.data.newFeeE6.to()),
                        protocol_swap_fee_e6: None
                    }
                })
            }
        }

        updates
    }
}

pub async fn fetch_l2_pools<P>(
    mut deploy_block: u64,
    end_block: u64,
    angstrom_v2_factory: Address,
    db: &P
) -> Vec<PoolKeyWithFees<L2FeeConfiguration>>
where
    P: Provider<Optimism>
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
            .address(angstrom_v2_factory);

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

    let all_updates = logs.into_iter().filter_map(|log| {
        let block_number = log.block_number.unwrap();

        if let Ok(event) = AngstromL2Factory::PoolCreated::decode_log(&log.inner) {
            let pool_key = event.key.clone();

            let pool_id = PoolId::from(PoolKey::from(pool_key.clone()));

            Some(PoolUpdate::ChainSpecific {
                pool_id,
                update: L2PoolUpdate::NewPool {
                    pool_id,
                    token0: pool_key.currency0,
                    token1: pool_key.currency1,
                    hook: event.hook,
                    tick_spacing: pool_key.tickSpacing.as_i32(),
                    block: block_number,
                    hook_fee: pool_key.fee.to(),
                    creator_tax_fee_e6: event.creatorTaxFeeE6.to(),
                    protocol_tax_fee_e6: event.protocolTaxFeeE6.to(),
                    creator_swap_fee_e6: event.creatorSwapFeeE6.to(),
                    protocol_swap_fee_e6: event.protocolSwapFeeE6.to()
                }
            })
        } else if let Ok(event) = AngstromL2Factory::ProtocolSwapFeeUpdated::decode_log(&log.inner)
        {
            let pool_id = PoolId::from(PoolKey::from(event.key.clone()));

            Some(PoolUpdate::FeeUpdate {
                pool_id,
                block: block_number,
                update: L2FeeUpdate {
                    protocol_tax_fee_e6:  None,
                    protocol_swap_fee_e6: Some(event.data.newFeeE6.to())
                }
            })
        } else if let Ok(event) = AngstromL2Factory::ProtocolTaxFeeUpdated::decode_log(&log.inner) {
            let pool_id = PoolId::from(PoolKey::from(event.key.clone()));

            Some(PoolUpdate::FeeUpdate {
                pool_id,
                block: block_number,
                update: L2FeeUpdate {
                    protocol_tax_fee_e6:  Some(event.data.newFeeE6.to()),
                    protocol_swap_fee_e6: None
                }
            })
        } else {
            None
        }
    });

    let chain_updates = all_updates
        .filter(|update| match update {
            PoolUpdate::FeeUpdate { .. } => true,
            PoolUpdate::ChainSpecific { update, .. } => match update {
                L2PoolUpdate::NewPool { .. } => true
            },
            _ => false
        })
        .sorted_by_key(|update| match update {
            PoolUpdate::FeeUpdate { block, .. } => -1 * *block as i64,
            PoolUpdate::ChainSpecific { update, .. } => match update {
                L2PoolUpdate::NewPool { block, .. } => -1 * *block as i64
            },
            _ => unreachable!()
        });

    let mut pool_keys: HashMap<PoolId, PoolKeyWithFees<L2FeeConfiguration>> = HashMap::new();

    chain_updates.for_each(|update: PoolUpdate<Optimism>| match update {
        PoolUpdate::FeeUpdate { pool_id, update: cfg_update, .. } => {
            if let Some(pool) = pool_keys.get_mut(&pool_id) {
                if let Some(fee) = cfg_update.protocol_swap_fee_e6 {
                    pool.fee_cfg.protocol_swap_fee_e6 = fee;
                }

                if let Some(fee) = cfg_update.protocol_tax_fee_e6 {
                    pool.fee_cfg.protocol_tax_fee_e6 = fee;
                }
            }
        }
        PoolUpdate::ChainSpecific { update, .. } => match update {
            L2PoolUpdate::NewPool {
                pool_id,
                token0,
                token1,
                creator_tax_fee_e6,
                protocol_tax_fee_e6,
                creator_swap_fee_e6,
                protocol_swap_fee_e6,
                tick_spacing,
                hook_fee,
                hook,
                ..
            } => {
                let pool_key_with_fees = PoolKeyWithFees {
                    pool_key: PoolKey {
                        currency0:   token0,
                        currency1:   token1,
                        fee:         U24::from(hook_fee),
                        tickSpacing: I24::unchecked_from(tick_spacing),
                        hooks:       hook
                    },
                    fee_cfg:  L2FeeConfiguration {
                        is_initialized: true,
                        creator_tax_fee_e6,
                        protocol_tax_fee_e6,
                        creator_swap_fee_e6,
                        protocol_swap_fee_e6
                    }
                };
                pool_keys.insert(pool_id, pool_key_with_fees);
            }
        },
        _ => unreachable!()
    });

    pool_keys.values().cloned().collect()
}
