---
pipeline: bug
slug: l2-swap-fee-mismatch
created: 2026-03-05T00:00:00Z
current_stage: execute
stages_completed:
  - stage: plan
    timestamp: 2026-03-05T00:00:00Z
  - stage: execute
    timestamp: 2026-03-05T00:00:00Z
---

# Pipeline: L2 Swap Fee Mismatch

Local swap simulation uses swap_fee=0 for L2 pools, but on-chain V4 applies the pool's static LP fee (e.g., 160 = 0.016%).
