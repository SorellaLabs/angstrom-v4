---
pipeline: l2-mev-tax-underflow
---

# Decisions

## 2026-03-06: Investigation Priority
**Question**: What should I prioritize in this bug investigation?
**Answer**: Root cause analysis — deep dive to find the fundamental issue causing the arithmetic underflow in the L2 hook.

## 2026-03-06: Investigation Scope
**Question**: Which components should I investigate?
**Answer**: Reported component only — focus on the MEV tax test + hook interaction at l2_revm_swap_test.rs:350.
