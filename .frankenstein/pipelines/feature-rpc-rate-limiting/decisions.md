---
pipeline: feature-rpc-rate-limiting
---

# Decisions

## 2026-03-06: Approach Selection
**Question**: Which approach to reduce RPC pressure?
**Answer**: Increase ticks_per_batch
**Rationale**: Zero library changes, test-only fix. Reduces calls from 1,200 to ~40 per pool.
