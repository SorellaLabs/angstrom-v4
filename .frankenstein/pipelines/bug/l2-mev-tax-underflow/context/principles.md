# Cached Principles

## Global
1. **Never silently default on failure paths** — Every abstraction must be 100% correct; if it cannot guarantee correctness, it must crash or propagate the error explicitly
2. **Test the live production flow, never shortcut** — Tests must exercise actual production code paths, never bypass or stub out real processing

## Rust
3. **Fix Clippy warnings properly** — Never suppress with `#[allow(clippy::...)]`, always fix the underlying issue
4. **Inline short statements** — When a code block is less than 30 chars wide, inline it
