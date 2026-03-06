# Cached Principles

## Global (2)

1. **Never silently default on failure paths** — Every abstraction must be 100% correct and 100% certain about what it returns — if it cannot guarantee correctness, it must crash or propagate the error explicitly.
2. **Test the live production flow, never shortcut** — Tests must exercise the actual production code paths that handle data — never bypass, stub out, or shortcut the real processing pipeline.

## Rust (2)

3. **Fix Clippy Warnings Properly** — Never fix a clippy warning by adding `#[allow(clippy::...)]` — always address the underlying issue.
4. **Inline Short Statements** — When a code block or assignment is less than 30 characters wide, inline it onto a single line.
