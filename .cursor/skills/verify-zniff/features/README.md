# zniff verification map

Read this index, then the feature file. The GUI always uses the real data directory, so Doctor must pass before `make dev`. HTTP proofs use the library tests, which bind `127.0.0.1:0`.

## Baseline preconditions

- Repo root is the current directory.
- `cargo test --manifest-path app/Cargo.toml --offline` can compile.
- `.cursor/skills/verify-zniff/doctor.sh` has been run. Exit 0 is required only for a window launch.
- Never start a second zniff, and never press Start, while a user capture is running.

## Driving conventions

- HTTP pages: `cargo test --manifest-path app/Cargo.toml --offline --lib -- --test-threads=1 <test name>`.
- Window, only when Doctor exits 0: `make dev`.
- Do not call `networksetup` or `security`.
- Leave files under `evidence/` after cleanup.

## Proof and skip reporting

- Record the command, the exit code, and the assertion that names the user-visible result.
- If Doctor exits 1, do not claim the window was verified. Say which check failed.
- A skipped entry point is not verified by a different path.

## Features

- [Install page](./install-page.md) is the phone certificate page at `/zniff`.
- [Share link](./share-link.md) is the timed UUID page and its JSON.
- [Filter box](./filter-box.md) is the list filter, chips, and saved rules.
