use alloy_network::Ethereum;
use alloy_provider::Provider;
use futures::Stream;
use uni_v4_common::{PoolUpdate, V4Network};

pub mod completed_block_stream;
pub mod update_provider;
use uni_v4_structure::{PoolId, PoolKeyWithFees};

use crate::pool_providers::update_provider::PoolUpdateError;

pub trait PoolEventStream<T: V4Network>:
    Stream<Item = Vec<PoolUpdate<T>>> + Send + Unpin + 'static
{
    fn start_tracking_pool(&mut self, pool_id: PoolId);
    fn stop_tracking_pool(&mut self, pool_id: PoolId);
    fn set_pool_registry(&mut self, pool_registry: T::PoolRegistry);
}

pub trait ProviderChainUpdate<T: V4Network> {
    fn fetch_chain_data(
        &mut self,
        from_block: u64,
        to_block: u64
    ) -> impl Future<Output = Result<Vec<PoolUpdate<T>>, PoolUpdateError>> + Send;
}

pub trait ProviderChainInitialization<T: V4Network>: Provider<T> {
    fn fetch_pools(
        &self,
        address_book: T::AddressBook,
        start_block: u64,
        end_block: u64
    ) -> impl Future<Output = Result<Vec<PoolKeyWithFees<T::FeeConfig>>, PoolUpdateError>> + Send;
}

impl<P> ProviderChainInitialization<Ethereum> for P
where
    P: Provider<Ethereum>
{
    async fn fetch_pools(
        &self,
        address_book: <Ethereum as V4Network>::AddressBook,
        start_block: u64,
        end_block: u64
    ) -> Result<Vec<PoolKeyWithFees<<Ethereum as V4Network>::FeeConfig>>, PoolUpdateError> {
        Ok(crate::pool_providers::update_provider::fetch_angstrom_pools(
            start_block,
            end_block,
            address_book.angstrom,
            address_book.controller_v1,
            self
        )
        .await)
    }
}
