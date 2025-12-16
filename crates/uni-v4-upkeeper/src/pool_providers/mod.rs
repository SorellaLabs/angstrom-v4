use futures::Stream;
use uni_v4_common::{PoolUpdate, V4Network};

pub mod completed_block_stream;
pub mod update_providers;
use uni_v4_structure::PoolId;

use crate::pool_providers::update_providers::PoolUpdateError;

pub trait PoolEventStream<T: V4Network>:
    Stream<Item = Vec<PoolUpdate<T>>> + Send + Unpin + 'static
{
    fn start_tracking_pool(&mut self, pool_id: PoolId);
    fn stop_tracking_pool(&mut self, pool_id: PoolId);
    fn set_pool_registry(&mut self, pool_registry: T::PoolRegistry);
}

pub trait ProviderChainUpdate<T: V4Network> {
    async fn fetch_chain_data(
        &mut self,
        from_block: u64,
        to_block: u64
    ) -> Result<Vec<PoolUpdate<T>>, PoolUpdateError>;
}
