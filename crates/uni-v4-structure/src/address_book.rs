use alloy_primitives::Address;

#[derive(Debug, Clone, Copy)]
pub struct L1AddressBook {
    pub controller_v1: Address,
    pub angstrom:      Address
}

impl L1AddressBook {
    pub fn new(controller_v1: Address, angstrom: Address) -> Self {
        Self { controller_v1, angstrom }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct L2AddressBook {
    pub angstrom_v2_factory: Address
}

impl L2AddressBook {
    pub fn new(angstrom_v2_factory: Address) -> Self {
        Self { angstrom_v2_factory }
    }
}
