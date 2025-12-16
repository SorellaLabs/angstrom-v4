use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll}
};

use alloy_consensus::{BlockHeader, Transaction};
use alloy_eips::BlockId;
use alloy_network::{BlockResponse, Ethereum, Network};
use alloy_primitives::{Address, U160, aliases::I24};
use alloy_provider::Provider;
use alloy_rpc_types::{Block, Filter};
use alloy_sol_types::{SolCall, SolEvent};
use futures::{FutureExt, StreamExt, stream::Stream};
use op_alloy_network::Optimism;
use thiserror::Error;
pub use types::*;
use uni_v4_common::{ModifyLiquidityEventData, PoolUpdate, StreamMode, SwapEventData, V4Network};
use uni_v4_structure::{PoolId, PoolKey, updates::l2::L2PoolUpdate};

use crate::{
    pool_data_loader::{DataLoader, IUniswapV4Pool, PoolDataLoader},
    pool_providers::{
        PoolEventStream, ProviderChainUpdate,
        update_providers::{PoolUpdateError, PoolUpdateProvider}
    }
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

        // updates.extend(
        //     self.fetch_controller_batch_updates(from_block, to_block)
        //         .await?
        // );

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

                // Get the Uniswap pool ID from registry
                let angstrom_pool_id = PoolId::from(pool_key);
                let pool_id = self
                    .pool_registry
                    .private_key_from_public(&angstrom_pool_id)
                    .unwrap();

                updates.push(PoolUpdate::ChainSpecific {
                    pool_id,
                    update: L2PoolUpdate::NewPool {
                        pool_id,
                        token0: pool_key.currency0,
                        token1: pool_key.currency1,
                        tick_spacing: pool_key.tickSpacing.as_i32(),
                        block: block_number,
                        creator_tax_fee_e6: event.creatorTaxFeeE6.to(),
                        protocol_tax_fee_e6: event.protocolTaxFeeE6.to(),
                        creator_swap_fee_e6: event.creatorSwapFeeE6.to(),
                        protocol_swap_fee_e6: event.protocolSwapFeeE6.to()
                    }
                });
            }
        }

        updates
    }
}
