# zniff 竞品差距分析与下一阶段演进路线图

- **评估基线**：`zniff v0.4.0`（`nsh 0.6.1`，单进程 Rust 桌面端 + 同构 Wasm `zniff --web` 操作台 + `zniff --relay`）
- **决策来源**：基于当前源码能力盘点（见 [`docs/known-issues.md`](known-issues.md)、[`docs/open-questions.md`](open-questions.md)、[`docs/feature-optimization-analysis.md`](feature-optimization-analysis.md)）与用户对齐后的演进结论

---

## 一、产品定位与核心边界（Positioning & Non-Goals）

`zniff` 已完成核心抓包、规则调试、多端同步、AI 联动与轻量扩展基线建设。继续坚持以下定位与约束：

- **核心定位**：**轻量单进程、内存严格有界（16 KB 阈值环形分片 `zniff-spill` 落盘）、纯文本规则易分享、开箱即用**；同时通过内置本地 MCP Server 与零配置接入降低日常联调启动摩擦。
- **模块组织约束**：遵守 [`AGENTS.md`](../AGENTS.md) 单文件规模规范，新增子系统均拆分为独立子模块（目标 `<= 600` 行，建议上限 `800` 行，硬性上限 `1200` 行）；代理内核已拆分为 [`app/src/proxy/`](../app/src/proxy/) 子模块，不向 `ca.rs`、`present.rs` 或 `lib.rs` 继续堆砌成块新子系统。
- **明确边界（Non-Goals）**：
  - **不做成重型 Postman / 团队 API 协作平台**：不引入多层级集合目录树、账号体系或云端同步。
  - **不做网卡级 TUN/TAP 透明代理与 HTTP/3 / QUIC 拦截**：保持显式 HTTP 代理模型。
  - **不内嵌重度 C/C++ 脚本运行时（如 Node / Python / C 版 QuickJS）**：避免破坏 macOS / Windows / Linux / `wasm32` / Docker 静态交叉编译的确定性与轻量二进制体积。
  - **不做高复杂度安全 Fuzzer / 漏洞扫描器**。

---

## 二、与主流同类产品的能力横评矩阵

对比当前主流抓包与 API 调试工具：**Proxyman**、**HTTP Toolkit**、**Reqable**、**mitmproxy**、**Charles**。

| 能力维度 | zniff (`v0.4.0`) | Proxyman | HTTP Toolkit | Reqable | mitmproxy | Charles |
|---|---|---|---|---|---|---|
| **架构与轻量度** | ✅ 原生 Rust + `nsh 0.6`（内存有界分片） | 原生 Swift（仅 macOS，Windows 另写） | Electron（体积大、内存高） | Flutter + C++ 引擎 | Python（Terminal/Web） | Java JVM（较笨重） |
| **同构 Web / 远程操作台** | ✅ `--web` 共用同份 Rust→Wasm 界面 + `--relay` | ⚠️ 无完整同构控制台 | ❌ 本地桌面为主 | ❌ 移动端协同 app | ✅ `mitmweb` | ❌ 无 |
| **AI 原生 / MCP** | ✅ 内置 `POST /mcp` 与 `--mcp`（10 个 Tools，含 `generate_openapi`） | ❌ 无原生 MCP | ❌ 无 | ⚠️ 有限辅助 | ⚠️ 需外部脚本 | ❌ 无 |
| **隐私脱敏与静态分享** | ✅ HAR/HTML/实时分享 + 结构化 `multipart` + `redact_strict` | ❌ 无结构化严格遮罩 | ❌ 仅本地导出 | ❌ 仅基础导出 | ❌ 需脚本手写 | ❌ 无 |
| **纯文本可分享规则** | ✅ 17 类规则（16 项快捷标签）+ OR / `op=` / `#` 单行启停与表单回填 / Chunked 改写 | UI 规则列表（JSON 导入导出） | UI 规则匹配器 | 重写规则 + Python 脚本 | Python 脚本 | UI Rewrite / Map |
| **零配置单终端/单浏览器隔离** | ✅ **独立 SPKI Chromium + 预注入代理终端 + `adb` 一键反代** | ✅ 独立终端/iOS/Android 脚本 | ✅ **极佳（独立浏览器/终端/adb）** | ✅ 协同手机端 + 脚本 | ⚠️ 手配环境变量 | ❌ 主要改全局代理 |
| **动态签名 / 自定义加解密** | ✅ **内置 `{{hmac_sha256}}` 模板变量 + 本地 HTTP `hook`（`mutate` / `preview`）** | ✅ JS 脚本钩子 | ⚠️ 收费脚本步骤 | ✅ Python 脚本 | ✅ **极强 Python 插件** | ❌ 弱 |
| **Schema 级协议解码** | ✅ **支持 `.proto` Schema 挂载解码（零 `protoc` 与外部 crate 依赖）+ JSON/MsgPack 树形视图** | ✅ 支持 `.proto` 解析 | ✅ 支持协议拓展 | ⚠️ 基础预览 | ✅ Protobuf 插件 | ✅ 支持 `.proto` 映射 |
| **从抓包沉淀 Mock / OpenAPI** | ✅ **一键生成本地 Mock 目录（`fallback`）+ 导出 OpenAPI 3.0.3（JSON/YAML/复制）** | ✅ 导出 OpenAPI / Mock | ✅ 从流量建 Mock | ✅ API 集合 + Mock | ❌ 需插件 | ❌ 仅单条 Map Local |

### 小结：阶段交付完成状态

Phase 1 至 Phase 3b 规划的四大演进方向已**全部落地并完成回归验收**：

1. **核心基线优势**：单二进制跨平台轻量体验、环形分片内存治理、同构 `--web` / `--relay` 分享、企业级结构化脱敏预览、内置 10 个 MCP Tools（含 `generate_openapi`）与纯文本可分享规则体系。
2. **零配置接入三件套（`launcher.rs`）**：无需修改系统代理或钥匙串即可拉起独立 SPKI Chromium、预注入代理终端与 Android `adb` 一键反代（显示已挂接设备数并支持一键断开还原）。
3. **两层零 C 动态扩展（`rule_vars.rs` + `webhook.rs`）**：支持 `{{hmac_sha256}}` 等内置动态模板变量，以及本机回环 HTTP `hook`（含转发期 `mode=mutate` 改写与详情页只读 `mode=preview` 解密预览）。
4. **Schema 解码与抓包资产导出（`proto_schema.rs` + `openapi.rs` + `mock_gen.rs`）**：纯 Rust 零依赖 `.proto` Schema 挂载解码、OpenAPI 3.0.3（JSON / YAML / 剪贴板复制）导出与批量本地 Mock 目录生成。

---

## 三、三大演进方向与技术架构设计

### 方向一（Phase 1）：极致「零配置接入」三件套（Zero-Config Interception） — ✅ 已落地

核心目标：**不动全局系统代理、不污染系统全局证书库**，一键打开即抓纯净流量。

- **新增模块**：`app/src/launcher.rs`（仅非 wasm32 目标 `#[cfg(not(target_arch = "wasm32"))]`），前端入口收敛于顶栏「更多」`免配抓包 [浏览器][终端][ADB]`（[`app/src/ui/more_menu_rows.rs`](../app/src/ui/more_menu_rows.rs)）。

#### 1.1 免装系统证书独立浏览器（Isolated Chromium Browser）

- **自动探测浏览器路径**：
  - macOS：Google Chrome、Microsoft Edge、Brave、Arc/Chromium（`/Applications/.../Contents/MacOS/...`）
  - Windows：`chrome.exe`、`msedge.exe`（注册表 / Program Files / LocalAppData）
  - Linux：`google-chrome`、`chromium`、`chromium-browser`、`microsoft-edge`
- **零弹窗 TLS 信任原理（复用 `Ca` 现有结构）**：
  - [`app/src/ca.rs`](../app/src/ca.rs) 中的 `Ca` 已在进程内固定持有根证书 `ca_der` 及共享的 ECDSA P-256 叶子私钥 `leaf_key_der`。
  - 在 `ca.rs` 新增辅助函数提取两者的 **SPKI SHA-256 Base64 指纹**，作为 Chromium 启动参数 `--ignore-certificate-errors-spki-list=<ca_spki>,<leaf_spki>` 传入。
  - 配合参数：
    - `--proxy-server=http://127.0.0.1:<当前监听端口>`
    - `--user-data-dir=<临时隔离目录>`（与用户日常浏览器配置、Cookie、扩展完全隔离）
    - `--no-first-run --no-default-browser-check`
  - 无需将 `zniff CA` 写入系统钥匙串，即可 100% 无警告抓包 HTTPS/HTTP2/WebSocket；记录子进程与临时目录，退出时按需清理。

#### 1.2 一键预注入代理环境的终端（Proxied Terminal）

- **复用环境变量集合**：直接复用 [`app/src/present.rs`](../app/src/present.rs) 中 `proxy_env_export` 的全量变量（`http_proxy`、`https_proxy`、`all_proxy` 大小写、`SSL_CERT_FILE`、`NODE_EXTRA_CA_CERTS`、`REQUESTS_CA_BUNDLE`、`CURL_CA_BUNDLE`、`GIT_SSL_CAINFO`、`CARGO_HTTP_CAINFO`、`DENO_CERT`、`JAVA_TOOL_OPTIONS`），并附加提示标识如 `PS1` / 终端标语 `(zniff:<port>)`。
- **多平台终端拉起**：
  - macOS：优先使用探测到的 `iTerm.app` / `Ghostty` / `Warp` / 系统 `Terminal.app`，写入一次性临时 env/init 脚本后拉起新窗口并自清理脚本。
  - Windows：优先 `wt.exe`（Windows Terminal），回退 `powershell.exe -NoExit -Command ...`。
  - Linux：按顺序探测 `x-terminal-emulator`、`gnome-terminal`、`konsole`、`kitty`、`alacritty`。

#### 1.3 Android 设备 / 模拟器 `adb` 一键代理（与现有 iOS Simulator 对齐）

- **对称复用生命周期设计**：参考 [`ca.rs`](../app/src/ca.rs) 已实现的 `xcrun simctl` iOS 模拟器接入与自动清理机制：
  - 扫描 `adb devices` 在线设备序列号；
  - 对选中或全部在线设备执行：
    - `adb -s <serial> reverse tcp:<port> tcp:<port>`（USB 线直连转发回环端口，无需同一局域网 Wi-Fi 也能抓）；
    - `adb -s <serial> shell settings put global http_proxy 127.0.0.1:<port>`；
  - 在弹窗/状态提示引导或推送 `ca.crt` 至 `/sdcard/Download/zniff-ca.crt`（并附带 `network_security_config` 与系统安装指引）；
  - 进程内记录已修改过代理的 `<serial>` 集合，点击「停止抓包」或应用退出时自动执行 `settings put global http_proxy :0` 与 `adb reverse --remove tcp:<port>` 还原网络。

---

### 方向二（Phase 2）：两层渐进式「轻量动态扩展」（零 C 依赖）

用极简成本解决 80% 静态签名计算问题，并用语言无关的本地 Webhook 解决剩下 20% 复杂加解密，**不向二进制引入任何 C/C++ 脚本虚拟机**。

#### 2.1 第一层：规则内置动态模板变量与签名函数（`app/src/rule_vars.rs`） — ✅ 已落地

在保留纯文本 DSL 的前提下，让 `header`、`query`、`json`、`replace` 参数值在代理转发时支持 `{{...}}` 动态求值（含 `+` 拼接）：

- **基础与随机变量**：`{{uuid}}`、`{{ts}}`（秒级时间戳）、`{{ts_ms}}`（毫秒级时间戳）、`{{iso_now}}`、`{{random_int(min,max)}}`
- **当前请求/响应上下文引用**：`{{req.method}}`、`{{req.host}}`、`{{req.path}}`、`{{req.target}}`、`{{req.query.<key>}}`、`{{req.header.<name>}}`、`{{req.body}}`、`{{res.status}}`、`{{res.header.<name>}}`、`{{res.body}}`
- **纯 Rust 摘要与编码（零额外依赖实现 RFC 6234 SHA-256 与 RFC 2104 HMAC-SHA256）**：
  - `{{sha256(x)}}`、`{{sha256_b64(x)}}`、`{{hmac_sha256(secret, msg)}}`、`{{hmac_sha256_b64(secret, msg)}}`
  - `{{base64(x)}}`、`{{urlencode(x)}}`

规则示例：

```text
header api.vicanso.dev/v1 req set X-Timestamp {{ts_ms}}
header api.vicanso.dev/v1 req set X-Request-Id {{uuid}}
header api.vicanso.dev/v1 req set X-Signature {{hmac_sha256("dev-secret", req.body)}}
```

#### 2.2 第二层：本地 HTTP Webhook 扩展钩子（`app/src/webhook.rs`） — ✅ 已落地（`mode=mutate` + `mode=preview`）

针对自定义 AES/SM4/RSA 加密报文、多步 OAuth/Token 刷新等重度逻辑，允许用户用熟悉的 Python / Node / Go / Rust 跑一个本地服务挂载进来：

- **规则语法**：
  ```text
  hook <主机[/路径]> <http://127.0.0.1:port/endpoint> [req|res|both] [mode=mutate|preview] [timeout=2000ms] [op=<gql_op>]
  ```
- **已落地能力**：
  - `mode=mutate`（默认）：代理将 `<= 1 MB` 有界请求/响应 JSON `{id, phase, method, url, host/status, path?, headers, body_base64, body_utf8}` POST 到本地 hook，按返回的 `{path?, status?, headers?, body_base64? | body_utf8?, comment?}` 原地替换后再转发。
  - `mode=preview`：完全不改动转发字节，仅在详情页「美化 / 树形」视图非阻塞调用本地 hook（发送 `{id, phase: "req"|"res", mode: "preview", ..., body_b64}`，读取 `{body_b64? | body?, content_type?, note? | comment?}`）解密渲染并缓存结果，切回「原文」可对照原始密文。
- **安全与稳定性护栏**：
  - 默认仅允许向本机回环地址（`127.0.0.1` / `localhost` / `[::1]`）发起 hook 请求，防止配置误写导致流量外泄；
  - hook 客户端请求直连且不进入 `zniff` 自身代理与会话列表，杜绝自环死锁；
  - 统一受限于 `timeout` 与 `1 MB` `BODY_CAP`，超时按约定在备注记录警告后继续放行。

---

### 方向三（Phase 3）：`.proto` Schema 解码与一键生成 Mock / OpenAPI

#### 3.1 `.proto` Schema 挂载解码（`app/src/proto_schema.rs`） — ✅ 已落地

- **选型方案**：完全自研纯 Rust 零外部依赖的 `.proto`（proto2/proto3）词法/语法解析与 Wire 解码器，**无需本机安装 `protoc` 命令行、不引入额外第三方 crate 或 C 依赖**，原生与 `wasm32` 全平台均可编译。
- **规则与挂载方式**：
  ```text
  proto api.vicanso.dev/grpc ./protos [req=<pkg.ReqMsg>] [res=<pkg.ResMsg>]
  ```
- **gRPC 路径自动推断**：对标准的 gRPC `:path` `/package.Service/Method`，自动从加载的 `.proto` `service` 定义查找对应 RPC 方法的输入与输出消息，将二进制 Wire 载荷解码为带真实字段名、枚举名称、`oneof`、`map<K,V>` 与嵌套结构的 JSON。
- **UI 无缝衔接**：复用 [`app/src/ui/detail_body.rs`](../app/src/ui/detail_body.rs) 现有的「`美化 | 树形 | 原文`」切换条——配置好 `.proto` 后自动从通用 `field #N` Wire 视图升级为完整语义 JSON 与可折叠树形视图。

#### 3.2 一键生成本地 Mock 目录与 OpenAPI 3.0.3 导出（`app/src/mock_gen.rs` & `app/src/openapi.rs`） — ✅ 已落地

- **一键生成本地 Mock 目录（Batch Map Local）**：
  - 支持对「当前筛选结果」批量落盘为 `<output_dir>/<host>/<path>[.<METHOD>].json` 静态目录结构（自动遵守当前 `redact_export` / `redact_strict` / `redact_fields` 脱敏设定），并在 `rules::resolve_map_local_target` 中扩展 `index.json` 与 `<seg>.json` 回退解析；
  - 生成完成后一键追加对应 `map-local <host> "<output_dir>" fallback` 规则，让前后端联调从「依赖不稳定的测试环境」秒切为「可离线复现的本地 Mock 套件」。
- **一键导出 OpenAPI 3.0.3（JSON / YAML / 剪贴板复制）**：
  - 按 `host + method + normalized_path` 自动聚类（自动识别纯数字 ID、UUID 路径段合并为 `{id}` 参数）；
  - 合并多次请求的 Query 参数、Header、Request JSON Schema 与按状态码分类的 Response JSON Schema（自动推断多样本可选字段）；
  - 已在顶栏「更多 -> `OpenAPI [JSON][YAML][复制]`」、Web API `GET /api/export.openapi.json` / `/api/export.openapi.yaml` 及第 10 个内置 MCP Tool `generate_openapi`（支持 `format: "json"|"yaml"`）中落地，让 AI 编码助手直接拿到准确的真实线上接口契约。

---

## 四、分阶段实施优先级与验收状态

| 阶段 | 核心交付 | 模块 | 状态 | 跨平台 / Wasm 边界 | 回归验收情况（参照 `zniff-regression`） |
|---|---|---|---|---|---|
| **Phase 1** | **零配置接入三件套**：免证书独立 Chromium、预注入代理终端、`adb` 一键代理 | `app/src/launcher.rs`（联动 `ca.rs`、`present.rs`、`more_menu_rows.rs`） | ✅ **已落地** | 仅桌面原生构建开放；Wasm UI 自动隐藏外部进程拉起入口 | Tier 1 全通过 + SPKI/env/adb 单测覆盖 |
| **Phase 2** | **两层轻量动态扩展**：`{{hmac_sha256}}` 等内置变量 + `hook` 本地 HTTP 钩子（`mutate` / `preview`） | `app/src/rule_vars.rs`、`app/src/webhook.rs`（联动 `rules.rs`、`proxy/`） | ✅ **已落地** | 纯 Rust 规则求值全平台可用；`webhook` 限本机回环调用 | 模板求值单测 + `smoke-proxy.sh` 动态签名与 `/__hook__` 实测通过 |
| **Phase 3a** | **抓包资产导出**：批量 Mock 目录导出（含 `.json`/`index.json` 回退）、OpenAPI 3.0.3（JSON/YAML/复制）+ 第 10 个 MCP Tool `generate_openapi` | `app/src/mock_gen.rs`、`app/src/openapi.rs`（联动 `rules.rs`、`mcp.rs`、`web.rs`、`more_menu_rows.rs`） | ✅ **已落地** | OpenAPI 生成兼容桌面与 Wasm；Mock 批量落盘限于桌面端 | 路径归并 `{id}` / Schema 合并 / Mock 导出单测 + `smoke-proxy.sh` `/api/export.openapi.json` & `generate_openapi` 实测 + Tier 1–3 验收 |
| **Phase 3b** | **`.proto` Schema 挂载解码** | `app/src/proto_schema.rs`（联动 `rules.rs`、`proxy/`、`detail_publish.rs`） | ✅ **已落地** | 纯 Rust 零外部依赖解析 `.proto` 与 gRPC 帧，无需安装 `protoc` | 嵌套消息/枚举/`map`/packed/gRPC RPC 单测 + Tier 1–2 全量回归通过 |

---

## 五、下一阶段演进规划（v0.4：降低切换门槛与竞品迁移闭环）

针对希望从 **Charles / Proxyman / Whistle / Reqable** 迁移到 `zniff` 的用户，本阶段坚持「单进程原生轻量、纯文本规则易分享、不做重型 Postman 集合树或 SQLite 持久化」的产品边界，按 **零摩擦上手 → 竞品规则与会话迁移 → 多组规则并行开关 → 连通性刚需与 AI 差异化** 四个波次分批推进：

### Wave 1：零摩擦上手与统一「接入中心」（Onboarding & Unified Connect Hub） — ✅ 已落地

**解决痛点**：当前「隔离浏览器 / 代理终端 / Android ADB」藏在顶栏 `更多` 菜单中段，「iOS 模拟器」在证书行下方，「手机扫码」在底栏，新用户首次打开且无流量时右栏只展示「还没有选择」与孤立的「构造请求」按钮，核心零配置抓包能力严重不可见。

- **双入口协同**：
  1. **无流量空态改造为「快速开始」引导卡片**（新增独立 UI 模块 `app/src/ui/onboarding_card.rs`，由 `detail_summary.rs` 装配）：
     - 仅在会话为空且未选中流量时展示；有流量后自动切回现有的「当前列表统计」卡片，不干扰日常高频视图。
     - 展示当前监听状态与根证书信任状态（未安装/未信任时提供一键安装/信任入口），并平铺 3 组即点即用动作：
       - **桌面免配抓包**：`[隔离浏览器]`（免系统证书）、`[代理终端]`、`[复制代理环境]`；
       - **移动端接入**：`[Android ADB]`（显示已挂接设备数 `ADB · N` 与一键断开）、`[iOS 模拟器]`（macOS）、`[手机扫码 / PAC]`；
       - **快速体验 / 迁移**：`[构造请求]`、`[导入规则或会话（Whistle / Charles / HAR / TOML）]`。
  2. **常驻紧凑「接入中心」弹窗**（新增 `app/src/ui/connect_hub.rs`，联动 `footer_qr.rs` / `toolbar.rs`）：
     - 在主界面常驻一个紧凑「接入 / Connect」入口（有流量后也能一秒打开，不再需要在长长的 `更多` 菜单里翻找）；
     - 一站收口桌面免配抓包（浏览器/终端/复制环境）、Android ADB（实时显示连接数、支持一键挂接与断开还原）、iOS 模拟器、Wi-Fi 局域网二维码及 `/proxy.pac` 自动配置地址（Wasm 端自动隐藏仅桌面可用的本机进程启动项）。

---

### Wave 2：竞品规则与会话零成本迁移导入（Whistle + Charles） — ✅ 已落地

**解决痛点**：老用户在 Whistle / Charles 里沉淀了大量环境转发、Mock 与改写规则，手动重写迁移成本高。

- **新增独立解析模块**：
  - `app/src/import_whistle.rs`（纯函数无外部依赖，桌面与 Wasm 通用）：
    - 支持直接拖入 `.txt` 规则文件或在导入框粘贴 Whistle 规则文本，自动转译为 `zniff` 规则语法：
      - Host/IP 映射（`api.example.com 127.0.0.1:8080`）→ `resolve`；
      - 远程转发（`http://` / `https://`）→ `map-remote`；
      - 本地文件/目录（`file://` / `xfile://`）→ `map-local`（`xfile://` 自动附 `fallback`）；
      - 头/参数/延迟/跨域/状态码/禁缓存/限速/替换（`reqHeaders://`、`resHeaders://`、`urlParams://`、`resDelay://`、`resCors://`、`statusCode://`、`disable://cache`、`resSpeed://`、`resReplace://`、`reqReplace://`、`tlsIntercept://` / `ignore://` 等）→ 对应 `zniff` 动词或直通名单提示；
      - 内联 `{value}` 块尽可能展开为单行 `header`/`replace`/`json` 规则；无法 1:1 映射的语法行**绝不静默丢弃**，统一以 `# [unmapped whistle] <原行>` 注释保留并计入转换摘要。
  - `app/src/import_charles.rs`（纯 Rust 轻量 XML / JSON 解析，零重依赖）：
    - 解析 **Charles XML 导出文件（`.xml`）**：支持 Map Local、Map Remote、Rewrite（Header/Query/Body Replace）、Breakpoints、DNS Spoofing（`resolve`）、No Caching（`no-cache`）、Throttling（`throttle`），保留禁用条目为 `#` 注释行并汇总转换报告；
    - 解析 **Charles JSON 会话（`.chlsj`）**：直接还原为 `zniff` 会话流量（方法、URL、状态码、请求/响应头、正文、耗时与 TLS 信息），与现有 `.har` 拖拽导入并列。
- **交互闭环**：拖拽 `.txt` / `.xml` / `.chlsj` / `.har` / `.toml` 到窗口（或在「导入」弹窗粘贴）时自动识别格式；规则导入前展示简洁转换摘要（已转换 N 条 / 注释保留 M 条），确认后作为一个独立规则分组（配合 Wave 3）或追加到当前规则。

---

### Wave 3：多组规则并行叠加启用（Multi-Group Rules） — ✅ 已落地

**解决痛点**：Whistle / Charles / Proxyman 用户习惯同时勾选多个独立规则组（例如 `[✓] 本地环境转发` + `[✓] 禁用缓存` + `[✓] 支付接口 Mock`），而当前 `SavedProfile` 一次只能激活一套配置，切换会整套覆盖。

- **模型设计与向后兼容**（`app/src/config.rs` + `app/src/rules.rs` + `app/src/ui/rule_groups.rs` + `app/src/ui/filter_panel.rs`）：
  - 在 `config.toml` 中新增 `[[rule_groups]]`（`name: String`, `enabled: bool`, `rules: String`，上限 `RULE_GROUP_CAP = 32`），与现有顶层 `rules` 和 `saved_profiles` **100% 向后兼容**；
  - 代理生效规则按「当前工作区 `rules`（最高优先级）+ 所有已启用的 `rule_groups`（按列表顺序）」拼接解析，同一套优先级模型（首个命中终点规则生效，修饰规则顺序累加）无需推翻重写；
  - 「过滤」弹出层规则编辑器上方提供紧凑的 **规则分组复选条**（一键勾选/取消、新建分组、删除分组、点击切换编辑器当前编辑的分组），顶栏状态摘要汇总所有已启用分组的规则总数；
  - 从 Wave 2 导入的 Whistle / Charles 规则自动创建或更新为一个独立命名分组并默认启用，不覆盖用户原有工作区规则。

---

### Wave 4：网络连通刚需、现代 AI 流式检视与轻量构造历史 — ✅ 已落地

1. **上游 SOCKS5 / SOCKS5h 代理支持**（`app/src/proxy/socks5.rs` + `app/src/proxy/upstream_io.rs`）：
   - 扩展 `upstream_proxy` 支持 `socks5://[user:pass@]host:port`（默认端口 `1080`）与 `socks5h://[user:pass@]host:port`（纯 Rust 实现 RFC 1928 握手与 RFC 1929 用户名密码认证，零额外 crate）；
   - `socks5h://` 强制将目标域名交给上游代理远程解析（避免本地 DNS 污染），解决使用 Clash / V2Ray / Sing-box / SSH 动态转发用户的核心连通性门槛。
2. **动态 `/proxy.pac` 自动配置脚本**（`app/src/bypass.rs` + `app/src/proxy/local_routes.rs`）：
   - 抓包端口新增 `GET/HEAD /proxy.pac` 与 `/zniff.pac`（`application/x-ns-proxy-autoconfig`，豁免 `proxy_auth`），结合当前 `bypass_hosts`（黑名单与 `only:` 白名单模式）动态生成 `FindProxyForURL(url, host)`，使回环地址与直通域名走 `DIRECT`；
   - 在「接入中心」与手机安装页 `/zniff` 直接提供 PAC 地址与一键复制按钮，大幅降低 iOS / macOS / Windows 系统级抓包对无关应用的干扰。
3. **LLM / AI SSE 流式聚合检视**（新增 `app/src/llm_stream.rs` + `app/src/ui/llm_view.rs` + 接入 `detail_headers_msgs.rs`）：
   - 自动识别 **OpenAI Chat Completions / Responses API、Anthropic Messages API、Google Gemini SSE** 及兼容接口的流式响应；
   - 在详情页「消息」卡片提供可切换的 **「AI 聚合 | 原始帧」** 视图：自动拼接完整文本输出、折叠展示 Reasoning/思考过程块、按索引重组增量 `tool_calls` JSON 参数，并在顶部摘要条显示 Provider、Model、TTFT（首 Token 延迟）、总耗时、Input/Output/Reasoning Token 用量及生成速率（`tok/s`）；
   - 严格纯解析渲染、零外网依赖，切回原始帧视图仍可逐帧过滤与复制排障。
4. **最近构造请求历史（最多 20 条，持久化）**（`app/src/config.rs` + `app/src/ui/compose_box.rs`）：
   - 持久化存储于 `config.toml` 的 `[[compose_history]]`（上限 `COMPOSE_HISTORY_CAP = 20`），重启后在「构造请求」右上角「最近 ▾」下拉自动恢复，支持一键回填重发与弹出层底部「清空历史」。
5. **分发减摩建议（工程外围）**：
   - 维护官方 Homebrew Tap（`brew install --cask zniff`）与 Windows Scoop / Winget 清单，降低新用户安装与升级门槛。

---

### v0.4 分波次实施优先级总览

| 波次 | 核心交付 | 主要模块 | 状态 | 跨平台 / Wasm 边界 |
|---|---|---|---|---|
| **Wave 1** | 无流量「快速开始」引导卡 + 常驻统一「接入中心」弹窗（浏览器/终端/ADB/iOS 模拟器/扫码） | `ui/onboarding_card.rs`、`ui/connect_hub.rs`、`ui/detail_summary.rs`、`ui/footer_qr.rs` | ✅ 已落地 | 桌面开放全部接入项；Wasm 自动隐藏本机进程启动项 |
| **Wave 2** | Whistle `.txt` + Charles `.xml` 规则转译导入 + Charles `.chlsj` 会话导入（拖拽 + 转换摘要） | `import_whistle.rs`、`import_charles.rs`、`ui/profile_io.rs` | ✅ 已落地 | 纯 Rust 解析，桌面与 Wasm 全平台可用 |
| **Wave 3** | 多组规则并行叠加开关（`[[rule_groups]]`，兼容 `SavedProfile`、`--web` 与 MCP） | `config.rs`、`rules.rs`、`ui/rule_groups.rs`、`ui/filter_panel.rs`、`proxy/hub.rs` | ✅ 已落地 | 全平台可用，MCP `get_rules`/`set_rules` 同步兼容 |
| **Wave 4** | 上游 `socks5://` / `socks5h://`、动态 `/proxy.pac`、LLM/AI SSE 流式聚合检视、持久化最近 20 条构造历史 | `proxy/socks5.rs`、`proxy/upstream_io.rs`、`bypass.rs`、`proxy/local_routes.rs`、`llm_stream.rs`、`ui/llm_view.rs`、`ui/compose_box.rs` | ✅ 已落地 | SOCKS5 与 PAC 仅服务端/桌面；LLM 视图与构造历史全平台可用 |
| **v0.4 收口强化** | 外部根证书探测复用（Whistle / mitmproxy / mkcert / Proxyman）、代理域名准入控制与 Docker 严格全拒、`TopBottom` 宽屏左右并排详情、Windows ARM64 支持 | `external_ca.rs`、`allow_hosts.rs`、`ui/access_popover.rs`、`ui/detail_summary.rs` | ✅ 已落地 | `allow_hosts` 覆盖桌面 / `--web` / Docker；外部 CA 探测仅桌面 |
