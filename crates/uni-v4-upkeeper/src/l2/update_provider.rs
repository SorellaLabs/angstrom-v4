use std::collections::{HashMap, HashSet};

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
use uni_v4_common::{PoolUpdate, V4Network};
use uni_v4_structure::{
    L2FeeConfiguration, PoolId, PoolKey, PoolKeyWithFees, fee_config::L2FeeUpdate,
    l2_structure::pool_updates::L2PoolUpdate, pool_registry::PoolRegistry
};

use crate::pool_providers::{
    ProviderChainInitialization, ProviderChainUpdate,
    update_provider::{PoolUpdateError, PoolUpdateProvider}
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
            event JITTaxStatusUpdated(address indexed hook, bool newJITTaxEnabled);
            event PriorityFeeTaxFloorUpdated(address indexed hook, uint256 newPriorityFeeTaxFloor);
            event WithdrawOnly();
        }

        #[derive(Debug)]
        #[sol(rpc)]
        contract AngstromL2Hook {
            function priorityFeeTaxFloor() external view returns (uint256);
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

/// Batch-fetch `priorityFeeTaxFloor` for a set of hook addresses.
async fn fetch_hook_floors<P: Provider<Optimism>>(
    provider: &P,
    hooks: HashSet<Address>
) -> HashMap<Address, u128> {
    let futures = hooks.into_iter().map(|hook_addr| async move {
        let hook = AngstromL2Hook::new(hook_addr, provider);
        let result = hook.priorityFeeTaxFloor().call().await.unwrap_or_else(|e| {
            panic!("Failed to read priorityFeeTaxFloor from hook {hook_addr:?}: {e}")
        });
        (hook_addr, result.to())
    });

    futures::future::join_all(futures)
        .await
        .into_iter()
        .collect()
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
        let logs = self.fetch_l2_factory_logs(from_block, to_block).await?;

        // Pre-scan for unique hook addresses from PoolCreated events
        let hook_addrs: HashSet<Address> = logs
            .iter()
            .filter_map(|log| {
                AngstromL2Factory::PoolCreated::decode_log(&log.inner)
                    .ok()
                    .map(|e| e.hook)
            })
            .collect();

        let hook_floors = fetch_hook_floors(self.provider(), hook_addrs).await;

        let updates = self.process_l2_factory_logs(logs, &hook_floors);
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
            .provider()
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
        logs: Vec<alloy_rpc_types::Log>,
        hook_floors: &HashMap<Address, u128>
    ) -> Vec<PoolUpdate<Optimism>> {
        // Pre-scan: collect hook-level state that may precede PoolCreated in
        // the same block. Without this, JITTaxStatusUpdated / WithdrawOnly
        // events emitted before PoolCreated would be lost because the pool
        // isn't in the registry yet when those events are processed.
        let mut hook_jit_tax: HashMap<Address, bool> = HashMap::new();
        let mut global_withdraw_only = false;
        for log in &logs {
            if let Ok(event) = AngstromL2Factory::JITTaxStatusUpdated::decode_log(&log.inner) {
                hook_jit_tax.insert(event.hook, event.data.newJITTaxEnabled);
            } else if AngstromL2Factory::WithdrawOnly::decode_log(&log.inner).is_ok() {
                global_withdraw_only = true;
            }
        }

        let mut updates = Vec::new();

        let registry = self.pool_registry_mut();

        for log in logs {
            let block_number = log.block_number.unwrap();

            if let Ok(event) = AngstromL2Factory::PoolCreated::decode_log(&log.inner) {
                let pool_key = event.key.clone().into();

                registry.add_new_pool(pool_key);

                let pool_id = PoolId::from(pool_key);
                let floor = hook_floors.get(&event.hook).copied().unwrap_or_else(|| {
                    panic!(
                        "Missing priorityFeeTaxFloor for hook {:?} — should have been pre-fetched",
                        event.hook
                    )
                });

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
                        protocol_swap_fee_e6: event.protocolSwapFeeE6.to(),
                        priority_fee_tax_floor: floor,
                        jit_tax_enabled: hook_jit_tax
                            .get(&event.hook)
                            .copied()
                            .unwrap_or(false),
                        withdraw_only: global_withdraw_only
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
                        protocol_tax_fee_e6:    None,
                        protocol_swap_fee_e6:   Some(event.data.newFeeE6.to()),
                        priority_fee_tax_floor: None,
                        jit_tax_enabled:        None,
                        withdraw_only:          None
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
                        protocol_tax_fee_e6:    Some(event.data.newFeeE6.to()),
                        protocol_swap_fee_e6:   None,
                        priority_fee_tax_floor: None,
                        jit_tax_enabled:        None,
                        withdraw_only:          None
                    }
                })
            } else if let Ok(event) =
                AngstromL2Factory::JITTaxStatusUpdated::decode_log(&log.inner)
            {
                let hook_address = event.hook;
                for (pool_id, _) in registry.pools(Some(hook_address)) {
                    updates.push(PoolUpdate::FeeUpdate {
                        pool_id,
                        block: block_number,
                        update: L2FeeUpdate {
                            protocol_tax_fee_e6:    None,
                            protocol_swap_fee_e6:   None,
                            priority_fee_tax_floor: None,
                            jit_tax_enabled:        Some(event.data.newJITTaxEnabled),
                            withdraw_only:          None
                        }
                    });
                }
            } else if let Ok(event) =
                AngstromL2Factory::PriorityFeeTaxFloorUpdated::decode_log(&log.inner)
            {
                let hook_address = event.hook;
                for (pool_id, _) in registry.pools(Some(hook_address)) {
                    updates.push(PoolUpdate::FeeUpdate {
                        pool_id,
                        block: block_number,
                        update: L2FeeUpdate {
                            protocol_tax_fee_e6:    None,
                            protocol_swap_fee_e6:   None,
                            priority_fee_tax_floor: Some(event.data.newPriorityFeeTaxFloor.to()),
                            jit_tax_enabled:        None,
                            withdraw_only:          None
                        }
                    });
                }
            } else if AngstromL2Factory::WithdrawOnly::decode_log(&log.inner).is_ok() {
                for (pool_id, _) in registry.pools(None) {
                    updates.push(PoolUpdate::FeeUpdate {
                        pool_id,
                        block: block_number,
                        update: L2FeeUpdate {
                            protocol_tax_fee_e6:    None,
                            protocol_swap_fee_e6:   None,
                            priority_fee_tax_floor: None,
                            jit_tax_enabled:        None,
                            withdraw_only:          Some(true)
                        }
                    });
                }
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
            .from_block(deploy_block)
            .to_block(this_end_block)
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

    // Pre-scan for unique hook addresses from PoolCreated events
    let hook_addrs: HashSet<Address> = logs
        .iter()
        .filter_map(|log| {
            AngstromL2Factory::PoolCreated::decode_log(&log.inner)
                .ok()
                .map(|e| e.hook)
        })
        .collect();

    let hook_floors = fetch_hook_floors(db, hook_addrs).await;

    // Track per-hook state for JIT tax and priority fee floor from events
    let mut hook_jit_tax: HashMap<Address, bool> = HashMap::new();
    let mut global_withdraw_only = false;

    // First pass: collect hook-level and factory-level settings
    for log in &logs {
        if let Ok(event) = AngstromL2Factory::JITTaxStatusUpdated::decode_log(&log.inner) {
            hook_jit_tax.insert(event.hook, event.data.newJITTaxEnabled);
        } else if AngstromL2Factory::WithdrawOnly::decode_log(&log.inner).is_ok() {
            global_withdraw_only = true;
        }
    }

    let all_updates = logs.into_iter().filter_map(|log| {
        let block_number = log.block_number.unwrap();

        if let Ok(event) = AngstromL2Factory::PoolCreated::decode_log(&log.inner) {
            let pool_key = event.key.clone();

            let pool_id = PoolId::from(PoolKey::from(pool_key.clone()));
            let floor = hook_floors.get(&event.hook).copied().unwrap_or_else(|| {
                panic!(
                    "Missing priorityFeeTaxFloor for hook {:?} — should have been pre-fetched",
                    event.hook
                )
            });

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
                    protocol_swap_fee_e6: event.protocolSwapFeeE6.to(),
                    priority_fee_tax_floor: floor,
                    jit_tax_enabled: hook_jit_tax
                        .get(&event.hook)
                        .copied()
                        .unwrap_or(false),
                    withdraw_only: global_withdraw_only
                }
            })
        } else if let Ok(event) = AngstromL2Factory::ProtocolSwapFeeUpdated::decode_log(&log.inner)
        {
            let pool_id = PoolId::from(PoolKey::from(event.key.clone()));

            Some(PoolUpdate::FeeUpdate {
                pool_id,
                block: block_number,
                update: L2FeeUpdate {
                    protocol_tax_fee_e6:    None,
                    protocol_swap_fee_e6:   Some(event.data.newFeeE6.to()),
                    priority_fee_tax_floor: None,
                    jit_tax_enabled:        None,
                    withdraw_only:          None
                }
            })
        } else if let Ok(event) = AngstromL2Factory::ProtocolTaxFeeUpdated::decode_log(&log.inner) {
            let pool_id = PoolId::from(PoolKey::from(event.key.clone()));

            Some(PoolUpdate::FeeUpdate {
                pool_id,
                block: block_number,
                update: L2FeeUpdate {
                    protocol_tax_fee_e6:    Some(event.data.newFeeE6.to()),
                    protocol_swap_fee_e6:   None,
                    priority_fee_tax_floor: None,
                    jit_tax_enabled:        None,
                    withdraw_only:          None
                }
            })
        } else {
            // PriorityFeeTaxFloorUpdated events are dropped — floor values come from
            // latest on-chain state via fetch_hook_floors() RPC calls above.
            // JITTaxStatusUpdated and WithdrawOnly events are dropped — their state
            // is collected in the first pass and applied during pool construction.
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
            PoolUpdate::FeeUpdate { block, .. } => *block as i64,
            PoolUpdate::ChainSpecific { update, .. } => match update {
                L2PoolUpdate::NewPool { block, .. } => *block as i64
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

                if let Some(floor) = cfg_update.priority_fee_tax_floor {
                    pool.fee_cfg.priority_fee_tax_floor = floor;
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
                priority_fee_tax_floor,
                jit_tax_enabled,
                withdraw_only,
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
                        protocol_swap_fee_e6,
                        priority_fee_tax_floor,
                        jit_tax_enabled,
                        withdraw_only
                    }
                };
                pool_keys.insert(pool_id, pool_key_with_fees);
            }
        },
        _ => unreachable!()
    });

    pool_keys.values().cloned().collect()
}

impl<P> ProviderChainInitialization<Optimism> for P
where
    P: Provider<Optimism>
{
    async fn fetch_pools(
        &self,
        address_book: <Optimism as V4Network>::AddressBook,
        start_block: u64,
        end_block: u64
    ) -> Result<Vec<PoolKeyWithFees<<Optimism as V4Network>::FeeConfig>>, PoolUpdateError> {
        Ok(fetch_l2_pools(start_block, end_block, address_book.angstrom_v2_factory, self).await)
    }
}
