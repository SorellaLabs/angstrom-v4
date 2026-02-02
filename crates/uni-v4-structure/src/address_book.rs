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
