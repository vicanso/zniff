---
name: verify-share-filter
description: "Check that a zniff live share document includes only flows matching the filter frozen when the link was created. Use when changing Hub::share_document, Session::flows_matching, or the share page filter."
---

# Share filter

A live share must not show flows the filter excluded. The harm is that a link shared under `header:cookie` also shows other hosts' requests.

## Property

`Hub::share_document` builds JSON from `Session::flows_matching` using the filter stored on `LiveShare`. A flow that does not match that filter is absent. The JSON still records the filter string.

This is a cargo test of the real function, not a separate model. Rust has no checker wired into this repo, and a hand model of `flows_matching` would drift from the parser. The test is the check.

## Run

From the repo root:

```bash
cargo test --manifest-path app/Cargo.toml --offline --lib -- --test-threads=1 share_document_keeps_only_the_frozen_filter
```

Exit 0 means the property holds for this fixture. Any failure is a bug until you can show the test no longer describes the code.

## What the fixture does

It opens two flows on a temp hub: `http://a.test/secret` with `Cookie: session=1`, and `http://b.test/other` with `Accept: text/plain`. It starts a share with `header:cookie`. The document must contain the first URL and must not contain the second.

## When it fails

If `share_document` uses `session.flows()` instead of `flows_matching`, the document contains `http://b.test/other`. That is a leak, not a modeling error. Do not delete the assertion to make the test pass.

## Scope

Checked: one `header:` filter, two flows, the share document JSON.

Not checked: every filter operator, the 400-flow cap, expiry, or the HTML page. A pass does not cover those.

## Coverage

- `app/src/proxy.rs` `Hub::share_document`
- `app/src/session.rs` `Session::flows_matching`
- `app/src/share.rs` `document`

Sync commit: `6f4fcfbf3c8b39ad1fa2870a193c022f06af895f`. Move this hash forward only after you confirm those three functions still match this description at the new commit.

## Keeping it current

After a change that touches the files above and does not rerun the command, use `/maintain-formal-verification-skill`.
