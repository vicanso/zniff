# Filter box

The filter field under the window title narrows the list. Top-bar 「过滤」edits the same text in a popover that stays open, and can save the text into `saved_filters`.

## Sub-features

- `filter-type` matches host or path, plus `header:`, `body:`, method, and status words together.
- `filter-save` stores the current text from the 「过滤」popover.
- `filter-live` updates the list while the popover stays open.

## How to get to it (user POV)

- Type in the filter field on the left.
- Press 「过滤」on the top bar, edit the field in the popover, then press 「保存当前过滤」.

## Driving it with cargo test

Preconditions:

- Doctor must pass before driving the window. There is no scripted click harness for nsh.
- The filter match itself is covered by session tests.

- **Header word.** Run `cargo test --manifest-path app/Cargo.toml --offline --lib -- --test-threads=1 filter_matches_body_and_header_keywords`. Exit 0. `header:x-trace` matches the flow that has that header and `body:1001 header:missing` matches nothing.
- **Window.** Only when Doctor exits 0: `make dev`, type `header:cookie` in the filter field, and confirm rows without that header disappear. Screenshot to `evidence/filter-box.png`.

## Gotchas

- Several words are AND.
- `header:` searches request and response header names and values. It does not search the query string. A bare word can match the path, which includes the query.
- `saved_filters` is separate from the automatic `recent_filters` list of 8.
- If Doctor fails, do not claim the window popover was verified.
