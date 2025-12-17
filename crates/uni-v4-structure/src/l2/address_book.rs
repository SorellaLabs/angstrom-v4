use alloy_primitives::Address;

#[derive(Debug, Clone, Copy)]
pub struct L2AddressBook {
    pub angstrom_v2_factory: Address
}

impl L2AddressBook {
    pub fn new(angstrom_v2_factory: Address) -> Self {
        Self { angstrom_v2_factory }
    }
}
