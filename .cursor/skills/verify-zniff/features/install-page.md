# Install page

A phone on the same LAN opens `http://{地址}:{端口}/zniff` and downloads the root certificate.

## Sub-features

- `install-open` returns the Chinese install page for `GET /zniff`.
- `install-cert-link` includes a link to `/zniff-ca.crt`.
- `install-not-proxied` does not treat `http://example.com/` as the local page.

## How to get to it (user POV)

- Start capture with LAN on, then open the address on the phone.
- On this computer, open `http://127.0.0.1:{端口}/zniff` while zniff is listening.

## Driving it with cargo test

Preconditions:

- Do not need Doctor for this test. It binds `127.0.0.1:0` and does not touch the system proxy.
- The user's zniff window must not be required.

- **Open the page.** Run `cargo test --manifest-path app/Cargo.toml --offline --lib -- --test-threads=1 install_page_links_to_the_certificate`. Exit 0. The test speaks `GET /zniff` to a real listener and the response contains `href="/zniff-ca.crt"`.
- **Leave other hosts alone.** Run `cargo test --manifest-path app/Cargo.toml --offline --lib -- --test-threads=1 local_routes_leave_proxied_roots_alone`. Exit 0. `http://example.com/` is not the install page.

## Gotchas

- `capturing = true` makes `make dev` bind the user's port. Use the test above instead of the window when Doctor fails.
- The install page is recorded as a flow. A share poll is not.
- `POST /zniff` is not the page.
