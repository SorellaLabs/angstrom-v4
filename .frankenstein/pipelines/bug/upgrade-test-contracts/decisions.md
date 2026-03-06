---
pipeline: upgrade-test-contracts
---

# Decisions

## 2026-03-06: Upgrade Approach
**Question**: Which approach for upgrading the test contracts?
**Answer**: Fix SwapQuoter with selector validation — update SwapQuoter.sol to check for QuoteResult selector before decoding, propagate unexpected reverts, then recompile and update embedded bytecode in both test files.
