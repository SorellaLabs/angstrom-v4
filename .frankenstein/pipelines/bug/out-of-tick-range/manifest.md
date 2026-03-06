---
pipeline: bug
slug: out-of-tick-range
created: 2026-03-05T00:00:00Z
current_stage: plan
stages_completed:
  - stage: plan
    timestamp: 2026-03-05T00:00:00Z
---

# Pipeline: Out of Tick Range on Large L2 Swap

Local swap panics with "out of initialized tick ranges loaded for uniswap" on CBBTC->ETH large swap (0.1 cbBTC). Root cause: batch advancement bug in tick loading gives only 12% of expected coverage.
