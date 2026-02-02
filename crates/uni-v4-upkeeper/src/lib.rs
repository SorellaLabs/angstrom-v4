#![allow(clippy::type_complexity)]

use alloy_primitives::I256;
use thiserror::Error;

pub mod baseline_pool_factory;
pub mod bindings;
pub mod pool_data_loader;
pub mod pool_manager_service;
pub mod pool_manager_service_builder;
pub mod pool_providers;
pub mod slot0;

#[cfg(feature = "l2")]
mod l2;
#[cfg(feature = "l2")]
pub mod l2_upkeeper {
    pub use super::l2::*;
}

fn i128_to_i256(value: i128) -> I256 {
    I256::try_from(value).unwrap()
}

#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("overflow from i32 to i24 {0:?}")]
    OverflowErrorI24(i32),
    #[error("overflow from I256 to I128 {0:?}")]
    OverflowErrorI28(I256)
}
