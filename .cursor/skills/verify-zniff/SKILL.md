---
name: verify-zniff
description: "Drive the zniff desktop capture app the way a user does: the nsh window when it is safe, and the proxy's own HTTP pages (install, share) through the library tests that bind 127.0.0.1:0. Use when proving a zniff UI or capture change, or when the user asks to verify zniff."
---

# Verify zniff

zniff is a desktop HTTP(S) capture app. The window is an nsh root component. The same process serves `/zniff` (phone install) and `/s/{uuid}` (a live share) on the proxy port. There is no browser harness and no separate data-dir flag. The GUI always uses `~/Library/Application Support/zniff`.

## Launch

Do not run `make dev` or the `zniff` binary when Doctor fails. `config.toml` `capturing = true` makes the next window press Start and bind the user's `listen_port` (often `0.0.0.0:8877`). That is the user's proxy, not a disposable instance.

When Doctor passes, the window command from the repo root is:

```bash
make dev
```

Ready means the nsh window is open and the status line is visible. Bacon keeps the console attached. Stop that process yourself when finished. `make dev` may steal `Cmd+R`.

The safe drive for HTTP pages does not open the window. From the repo root:

```bash
cargo test --manifest-path app/Cargo.toml --offline --lib -- --test-threads=1 <test>
```

Those tests bind `127.0.0.1:0`, open a real proxy listener, and speak HTTP. They must not call `networksetup` or `security`.

## Doctor

Run this first. Exit 0 means a window launch is allowed. Exit 1 means stop. Do not start a second zniff, and do not press Start.

```bash
.cursor/skills/verify-zniff/doctor.sh
```

It checks: no `zniff` process, `capturing` is false, `use_system_proxy` is false, and `proxy-snapshot.toml` `active` is false.

## Drive

Read `.cursor/skills/verify-zniff/features/README.md` and the feature file. Prefer the cargo test named in that file over the GUI when Doctor fails. A proof that only calls a setter is not a proof.

Stable handles:

- Top bar buttons: `清空`, `过滤`, `更多`, `开始` / `停止`.
- Filter field placeholder starts with `主机、路径`.
- Install page: `GET /zniff`, link `href="/zniff-ca.crt"`.
- Share page: `GET /s/{uuid}` and `GET /s/{uuid}/flows.json`. Absolute-form `http://example.com/s/...` is not local.

## Evidence

Write proof under `.cursor/skills/verify-zniff/evidence/`. Keep the command, stdout, and exit code. For the install page, the body must contain `href="/zniff-ca.crt"`. For a filter or share proof, the body must show the matching flow and must not show the excluded one. GUI proof is a screenshot of the window after the action, only when Doctor passed and you started that window. Cleanup must not delete this directory.

## Cleanup

Kill only the PID you started. Do not `pkill zniff`. Tests remove their own temp CA dirs. Leave `evidence/` in place.

```bash
# if you started make dev and recorded the PID
kill "$ZNIIF_PID"
```

## Helpers

`doctor.sh` is the read-only check. Run it as shown under Doctor.
