# zniff

桌面 HTTP(S) 抓包工具。界面基于 NSH 0.7 Rust 根组件，桌面端抓包代理与窗口同进程运行；`zniff --web` 复用同一套界面（编译为 `wasm`），但抓包只在服务端进程执行，仅记录把该进程设为 HTTP 代理的客户端。产品名全小写为 `zniff`。

`zniff-dev-plan.md` 仅为历史早期草案（gpui / hudsucker / 多 crate / SQLite / cyberapi），已废弃；实际实现与约束以 `app/` 代码、`.agents/rules/`、`docs/product-spec.md` 与 `docs/known-issues.md` 为准。

---

## 文档与规则索引

- **项目作用域规则（自动加载）**：
  - `.agents/rules/ui-ux.md`：UI 组件、NSH 0.7 陷阱、动效与文案、性能、`ui_zoom` 与 `/zniff` / `/s/{uuid}` 移动端规范
  - `.agents/rules/proxy-core.md`：代理内核、TLS/ALPN、内存与盘溢环、准入/规则、`zniff CA` 与系统代理保护
  - `.agents/rules/web-wasm.md`：`zniff --web` / `--relay` 端口与鉴权隔离、`wasm32` 禁区与测试约束
- **可迁移通用技能（跨 NSH 项目复用）**：
  - `.agents/skills/nsh-app-craft/SKILL.md`：通用 NSH 应用工程规范、UX 8 原则、Apple 级动效与 `reduce_motion()` 体系、`nsh 0.7 + wasm32` 避坑手册（可经 `npx skills add <repo> --skill nsh-app-craft -g` 安装到任何机器）
  - `.agents/skills/zniff-regression/SKILL.md`：3 层发布与合入前回归流程（Tier 1 静态/单测、Tier 2 实机代理 + MCP 冒烟、Tier 3 桌面 Retina 截图巡检）
- **产品与设计文档（按需查阅）**：
  - `docs/product-spec.md`：完整界面交互、列/过滤语法、详情视图、规则引擎、分享/中继、`--web` 与自动更新规格明细
  - `docs/known-issues.md`：已接受的能力缺口（新发现的已接受缺口补于此处）
  - `docs/open-questions.md`：产品或架构取舍待决项（记录于此，不因未定而阻塞当前开发）

---

## 工程布局与版本基线

- **单 crate 工作区**：`app/` 自带 `[workspace]`（入口 `app/src/main.rs`，根组件 `app/src/lib.rs::App`，清单 `app/nsh.json`）。禁止把仓库根也做成包含 `app/` 的 workspace。
- **NSH 版本联动**：依赖 `nsh 0.7`（`0.7.0`），Edition `2024`（`gen` 为保留字）。升级 `app/Cargo.toml` 的 `nsh-*` 版本时，必须同步更新 `Makefile` 的 `NSH_VERSION` 与 `Dockerfile` 的 `ARG NSH_VERSION`。
- **核心模块分布**：
  - `app/src/ui/` 与 `app/src/ui/kit/`：NSH 桌面/Web 共用界面与统一动效/按钮/浮层原语（`tokens.rs`、`button.rs`、`surface.rs`）
  - `app/src/proxy/`（`mod.rs`, `hub.rs`, `hub_session.rs`, `tls_mitm.rs`, `plain_http.rs`, `upstream_io.rs`, `local_routes.rs`, `replay.rs`, `wire.rs`, `hyper_forward.rs`, `hyper_helpers.rs`, `tests.rs`）与 `app/src/local_proc.rs`：抓包代理内核与回环进程识别
  - `macos/transparent-proxy/`、`linux/ebpf/`、`app/src/transparent_proxy*.rs`：桌面按进程透明抓包后端
  - `app/src/session.rs`、`ca.rs`、`sysproxy.rs`、`client_hello.rs`、`decompress.rs`：会话、CA 证书、系统代理、ClientHello 与解压
  - `app/src/web.rs`、`allow_hosts.rs`、`external_ca.rs`、`bypass.rs`、`rules.rs`、`rule_vars.rs`、`webhook.rs`、`proto_schema.rs`、`har.rs`、`share.rs`、`relay.rs`、`present.rs`、`openapi.rs`、`mock_gen.rs`、`launcher.rs`、`mcp.rs`、`ws.rs`、`updater.rs`、`ios_guide.rs`、`android_guide.rs`
  - `app/src/hub_web.rs`、`app/src/ca_web.rs`：仅 `wasm32` 替代本地 `proxy`/`ca`，经 `/api` 读写服务端
  - `app/src/config.rs`、`i18n.rs`、`icon.rs`：正式版读 `config.toml`，开发模式（`debug_assertions` 或 `ZNIFF_DEV=1`）隔离读写 `config.dev.toml`（默认端口 `8878`）与 `proxy-snapshot.dev.toml`；CA 共用

---

## 架构红线与代码复杂度门禁

- **明确非目标（除非用户明确要求，禁止新增）**：`HTTP/3`、`QUIC`、`SQLite`、`cyberapi`、`Firefox` 证书库、`hudsucker`；桌面窗口坚持使用原生 NSH，不另写一套独立 GUI。
- **单文件与函数复杂度红线**：
  - **拆分原则**：新增面板、弹窗、子视图或业务域必须拆为职责单一的独立子模块，根模块只做壳层组装与重导出。
  - **文件行数**：新增模块目标 `<= 600` 行，建议上限 `800` 行，**硬性上限 `1200` 行**（不含 `#[cfg(test)]`；测试超 `400` 行拆为独立 `tests.rs`）。
  - **函数复杂度**：单函数目标 `<= 80` 行，**硬性上限 `120` 行**；嵌套 `<= 4` 层；参数 `<= 7` 个（禁止继续为新函数加 `#[allow(clippy::too_many_arguments)]`）。
  - **历史大文件治理**：改动已超 1200 行的文件（`present.rs`、`share.rs`、`session.rs`、`rules.rs`、`web.rs`）时，禁止继续追加成块新系统，优先顺手抽离内聚子模块。
- **nsh 组件与设计规范优先原则**：新增或重构任何 UI 组件前，**必须先确认 `nsh`（`nsh_base` / `nsh_core` 本机源码或 `https://nsh.dev/elements/{name}.md`）是否已有对应原生组件**（如 `button`、`tabs`/`tab`、`select`、`sheet`、`resize_handle`、`collapsible`/`accordion`、`checkbox`、`radio_group`/`radio`、`switch`、`link`、`popover`、`menu`、`tooltip`、`data_table`、`command` 等），优先直接复用原生组件，不重复造轮子，也不用 `view().role(...)` 模拟标准控件（`popover()` / `menu()` 的 `.trigger(...)` 等要求 `View` 的宿主除外）；设计布局、间距密度、交互动效与键盘/无障碍规范时尽量参考 `nsh-design-guides` 与 `nsh` 官方设计/编码指南（查全局 `nsh` / `nsh-design-guides` skill 及本机源码），再叠加项目自身主题与桌面/Web 约束；窗口尺寸/坐标/模式经 `window_state()` 停半秒写入配置并在启动恢复（显示器消失时居中回落主屏）。

---

## 开发与验证命令

仓库根目录常用命令：

```bash
make fmt          # cargo fmt
make lint         # cargo clippy --all-targets -- -D warnings
make check-wasm   # wasm32-unknown-unknown 构建检查
make test         # cargo test
make web          # 增量编译 app/.nsh/web 并以 ZNIFF_DEV=1 启动 --web
make dev-app      # nsh run 启动桌面开发窗口
```

- **强制检查门禁**：每次改完 Rust 代码，必须跑 `make fmt` 与 `make lint`；若改动涉及 `app/src/ui/` 或 `#[cfg(target_arch = "wasm32")]` 代码，必须加跑 `cargo clippy --manifest-path app/Cargo.toml --target wasm32-unknown-unknown -- -D warnings` 与 `make check-wasm` 并确认全部 0 警告；涉及 CLI/`proxy`/`web`/`updater` 变动时加跑 `cargo check --manifest-path app/Cargo.toml --no-default-features`；提交前必须确保 `fmt`、双端 `clippy` 与 `make test` 全绿。
- **测试域名与环境纪律**：
  - 凡涉及域名的高亮/规则/过滤测试，统一使用 `vicanso.dev` 子域（如 `api.vicanso.dev`、`*.vicanso.dev`、`test.vicanso.dev`）。
  - 测试只绑 `127.0.0.1:0` 与临时 `--data-dir`；严禁在测试或自动化检查中执行 `networksetup`/`security` 或修改真实 `config.toml`、`ca.crt` 与系统代理。
  - 修改 UI 布局或视口逻辑后（除非用户不需要 GUI 截图），按 `zniff-regression` Tier 3 用临时 `--data-dir` 启动并检查对齐、对比度、窄窗与弹窗底部操作栏完整性；未被要求测试真实抓包时不要点「开始」，退出后核对无残留 `zniff` 进程且系统代理未被修改。
