# Decisions

## L1 Addresses
- Use mainnet addresses from `contracts/lib/angstrom/crates/types/src/primitive/contract/mod.rs` (chain_id=1)
- Angstrom: 0x0000000AA8c2Fb9b232F78D2B286dC2aE53BfAD4
- Controller V1: 0x16eD937987753a50f9Eb293eFffA753aC4313db0
- Pool Manager: 0x000000000004444c5dc75cB358380D2e3dE08A90
- Deploy Block: 22689729

## Block Sampling
- 100 consecutive blocks from a hardcoded recent starting block

## Pool Selection
- Auto-discover all pools via `auto_pool_creation=true`, test all that have liquidity

## Pool Discovery
- Discovered from historical events (not hardcoded pool IDs)

## Start Block
- Hardcoded recent block (user sets when running)
