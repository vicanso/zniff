---
trigger: always_on
---

# zniff Web Console、Relay 与 Wasm32 约束

- **进程与端口模型**：
  - `zniff --web`（`maybe_run_cli_mode` 早返回）共用同一 `App`（编译为 `wasm` 置入 `app/.nsh/web`），但抓包仅发生在服务端进程：网页端口启动即开启（默认 `127.0.0.1:8080`），抓包端口（默认 `8877`）仅在操作台点击开始后才绑定，绑失败保持关闭且不影响操作台访问。
  - `zniff --web` 不读写 `config.toml` 的 `capturing` 字段，不执行 `restore_if_dirty()`，也不调用任何系统代理 API；手机安装页 `/zniff`、证书 `/zniff-ca.crt`、分享 `/s/` 与中继 `/relay` 仍在抓包端口提供。
- **四端鉴权隔离**：
  - 操作台认证（`web_auth`）、抓包认证（`proxy_auth`）、中继上传（`Bearer` `relay_token`）与中继观看（`--view-auth`）四类凭据各管各的端口，严禁混用或跨端透传。
  - 若与 `--relay` / `--no-capture` 合跑，网页、抓包、中继端口不得冲突，并按既有参数互斥规则检查。
- **Wasm32 编码与兼容禁区**：
  - `app/src/hub_web.rs` 与 `app/src/ca_web.rs` 仅在 `target_arch = "wasm32"` 替代服务端代理与证书模块，Web 界面通过 `/api/state`、`/api/flows`、`POST /api/hub` 及 `/api/export.*` 通信，严禁在浏览器侧实现监听、解密或出现桌面专属操作项（系统代理、安装主机证书、复制代理环境、iOS 模拟器、自动更新、MCP）。
  - `wasm32` 代码严禁直接调用 `std::env::temp_dir()`、`std::process::id()`、`SystemTime::now()`、`Instant::now()`（会直接 panic），墙钟时间统一使用 `clock::now()`。
  - 不要将 `wasm` 产物编进桌面端二进制，不要覆盖用户本机 `~/.nsh`。
- **测试与构建门禁**：
  - Web 相关集成测试一律调用 `WebServer::start` 绑定 `127.0.0.1:0` 与临时 `--data-dir`；禁止调用 `run_cli`、`saved_config()`、绑定 `8877`、执行 `docker build` 或额外另起 `zniff` 进程。
  - 每次触碰 `app/src/ui/` 或任何 `#[cfg(target_arch = "wasm32")]` 分支，必须执行 `cargo clippy --manifest-path app/Cargo.toml --target wasm32-unknown-unknown -- -D warnings` 与 `make check-wasm`，确认双架构皆为 0 警告。
