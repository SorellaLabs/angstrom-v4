# Cached Principles

## Global
1. **Never silently default on failure paths** — Every abstraction must be 100% correct; crash or propagate errors, never silently default.
2. **Test the live production flow, never shortcut** — Tests must exercise actual production code paths, not simplified test-only paths.

## Rust
3. **Fix Clippy warnings properly** — Never suppress with `#[allow(clippy::...)]`; address the underlying issue.
4. **Inline short statements** — When a code block is <30 chars wide, inline it onto a single line.
