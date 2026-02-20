use alloy_primitives::aliases::{I24, U24};
use alloy_provider::Provider;
use futures::Stream;
use op_alloy_network::Optimism;
use uni_v4_common::PoolUpdate;
use uni_v4_structure::{
    L2FeeConfiguration, PoolId, PoolKey, l2_structure::pool_updates::L2PoolUpdate,
    pool_updates::Slot0Update
};

use crate::{
    baseline_pool_factory::{BaselinePoolFactory, UpdateMessage},
    pool_manager_service::{PoolEventProcessor, PoolManagerService},
    pool_providers::{PoolEventStream, ProviderChainInitialization},
    slot0::Slot0Stream
};

impl<P, Event, S> PoolEventProcessor<Optimism> for PoolManagerService<P, Optimism, Event, S>
where
    P: Provider<Optimism> + Clone + Unpin + 'static,
    Event: PoolEventStream<Optimism>,
    BaselinePoolFactory<P, Optimism>: Stream<Item = UpdateMessage<Optimism>> + Unpin,
    S: Slot0Stream,
    P: ProviderChainInitialization<Optimism>
{
    fn handle_chain_specific_update(&mut self, _: PoolId, update: &L2PoolUpdate) {
        match update {
            L2PoolUpdate::NewPool {
                pool_id,
                token0,
                token1,
                hook,
                hook_fee,
                tick_spacing,
                block,
                creator_tax_fee_e6,
                protocol_tax_fee_e6,
                creator_swap_fee_e6,
                protocol_swap_fee_e6,
                priority_fee_tax_floor,
                jit_tax_enabled,
                withdraw_only,
                ..
            } => {
                if self.auto_pool_creation {
                    let pool_key = PoolKey {
                        currency0:   *token0,
                        currency1:   *token1,
                        fee:         U24::from(*hook_fee),
                        tickSpacing: I24::unchecked_from(*tick_spacing),
                        hooks:       *hook
                    };
                    let fee_cfg = L2FeeConfiguration {
                        is_initialized:         true,
                        creator_tax_fee_e6:     *creator_tax_fee_e6,
                        protocol_tax_fee_e6:    *protocol_tax_fee_e6,
                        creator_swap_fee_e6:    *creator_swap_fee_e6,
                        protocol_swap_fee_e6:   *protocol_swap_fee_e6,
                        priority_fee_tax_floor: *priority_fee_tax_floor,
                        jit_tax_enabled:        *jit_tax_enabled,
                        withdraw_only:          *withdraw_only
                    };
                    self.handle_new_pool(pool_key, *block, fee_cfg);
                    tracing::info!("Pool configured: {pool_id:?}:\n{fee_cfg:?}");
                } else {
                    tracing::info!(
                        "Ignoring pool configured event (auto creation disabled): {:?}",
                        pool_id
                    );
                }
            }
        }
    }

    fn handle_slot0_updates(&mut self, _: Vec<Slot0Update>) {}

    fn dispath_chain_specific_update(&mut self, pool_id: PoolId, update: L2PoolUpdate) {
        match update {
            L2PoolUpdate::NewPool { .. } => {
                // CRITICAL: Process new pool to ensure it gets created in the factory
                // This will trigger pool data loading and initialization
                self.process_pool_update(PoolUpdate::ChainSpecific { pool_id, update });
            }
        }
    }
}
