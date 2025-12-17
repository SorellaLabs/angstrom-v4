use alloy_provider::Provider;
use futures::Stream;
use op_alloy_network::Optimism;
use uni_v4_common::PoolUpdate;
use uni_v4_structure::{
    L2FeeConfiguration, PoolId, l2_structure::pool_updates::L2PoolUpdate,
    pool_registry::PoolRegistry, pool_updates::Slot0Update
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
                block,
                creator_tax_fee_e6,
                protocol_tax_fee_e6,
                creator_swap_fee_e6,
                protocol_swap_fee_e6,
                ..
            } => {
                if self.auto_pool_creation {
                    let fee_cfg = L2FeeConfiguration {
                        is_initialized:       true,
                        creator_tax_fee_e6:   *creator_tax_fee_e6,
                        protocol_tax_fee_e6:  *protocol_tax_fee_e6,
                        creator_swap_fee_e6:  *creator_swap_fee_e6,
                        protocol_swap_fee_e6: *protocol_swap_fee_e6
                    };
                    // Reconstruct pool_key from the NewPool data
                    // We need to get the pool_key from the registry
                    if let Some(pool_key) = self.factory.registry().get(pool_id) {
                        self.handle_new_pool(*pool_key, *block, fee_cfg);

                        tracing::info!("Pool configured: {pool_id:?}:\n{fee_cfg:?}",);
                    } else {
                        tracing::warn!("Pool {:?} not found in registry", pool_id);
                    }
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
