# Package Manager Manifest Templates

This directory holds distribution manifest templates for installing **zniff** via package managers. All release assets and checksums are published alongside each GitHub Release (`SHA256SUMS`) by `.github/workflows/publish.yml`.

| Manager | Template | Target Repository |
| :--- | :--- | :--- |
| **Homebrew Cask** (macOS) | [`homebrew/zniff.rb`](./homebrew/zniff.rb) | `vicanso/homebrew-tap` (`Casks/zniff.rb`) or `homebrew/homebrew-cask` |
| **Scoop** (Windows) | [`scoop/zniff.json`](./scoop/zniff.json) | `vicanso/scoop-bucket` (`bucket/zniff.json`) or `ScoopInstaller/Extras` |
| **Winget** (Windows) | [`winget/vicanso.zniff.yaml`](./winget/vicanso.zniff.yaml) | `microsoft/winget-pkgs` (`manifests/v/vicanso/zniff/<version>/`) |

## Updating for a Release

1. Download `SHA256SUMS` from `https://github.com/vicanso/zniff/releases/download/v<VERSION>/SHA256SUMS`.
2. Update `version` / `PackageVersion` and replace the `REPLACE_WITH_SHA256_*` placeholders in the relevant manifest(s).
3. Verify locally before opening a PR:
   - **Homebrew**: `brew audit --cask --strict packaging/homebrew/zniff.rb`
   - **Scoop**: `scoop install ./packaging/scoop/zniff.json`
   - **Winget**: `winget validate --manifest packaging/winget/vicanso.zniff.yaml` (note: Winget portable archives require `.zip` Windows release assets when submitting to `microsoft/winget-pkgs`).
