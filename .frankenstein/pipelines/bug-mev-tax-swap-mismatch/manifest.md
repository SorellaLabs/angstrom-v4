---
pipeline: bug-mev-tax-swap-mismatch
type: bug
created: 2026-03-06T00:00:00Z
---

# Pipeline: bug-mev-tax-swap-mismatch

**Current Stage**: research
**Stages Completed**: research (2026-03-06)

## Summary
MEV tax swap test returns garbage on-chain values for small swap amounts. Root cause: SwapQuoter misinterprets non-QuoteResult reverts as amounts.
