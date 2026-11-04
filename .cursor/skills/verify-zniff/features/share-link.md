# Share link

「更多」→「分享 15 分钟」「分享 1 小时」or「分享 8 小时」copies `http://{地址}:{端口}/s/{uuid}`. The page refreshes from `/s/{uuid}/flows.json` and shows only flows matching the filter frozen at share time.

## Sub-features

- `share-filter` omits flows that do not match the frozen filter.
- `share-expired` answers 404 after the deadline or after 「停止分享」.
- `share-not-proxied` does not hijack `http://example.com/s/{uuid}`.

## How to get to it (user POV)

- Press Start, set a filter, then choose a share duration in 「更多」.
- Open the copied URL in a browser.

## Driving it with cargo test

Preconditions:

- The test binds `127.0.0.1:0`. Doctor is not required.

- **Filtered share.** Run `cargo test --manifest-path app/Cargo.toml --offline --lib -- --test-threads=1 share_document_keeps_only_the_frozen_filter`. Exit 0. The JSON contains `http://a.test/secret` and does not contain `http://b.test/other`.
- **Route.** `local_routes_leave_proxied_roots_alone` also asserts `/s/{uuid}` is local and `http://example.com/s/{uuid}` is not.

## Gotchas

- A new share replaces the previous id.
- Stopping capture ends the share.
- Without LAN, the copied host is `127.0.0.1`.
- The page lists at most 400 newer flows and 16 KB of each body.
