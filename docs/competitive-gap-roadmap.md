# zniff 竞品差距分析与下一阶段演进路线图

- **评估基线**：`zniff v0.3.0`（`nsh 0.6.0`，单进程 Rust 桌面端 + 同构 Wasm `zniff --web` 操作台 + `zniff --relay`）
- **决策来源**：基于当前源码能力盘点（见 [`docs/known-issues.md`](known-issues.md)、[`docs/open-questions.md`](open-questions.md)、[`docs/feature-optimization-analysis.md`](feature-optimization-analysis.md)）与用户对齐后的下一阶段演进结论

---

## 一、产品定位与核心边界（Positioning & Non-Goals）

`zniff` 已完成核心抓包、规则调试、多端同步与 AI 联动基线建设。下一阶段继续坚持以下定位与约束：

- **核心定位**：**轻量单进程、内存严格有界（16 KB 阈值环形分片 `zniff-spill` 落盘）、纯文本规则易分享、开箱即用**；同时通过内置本地 MCP Server 与零配置接入降低日常联调启动摩擦。
- **模块组织约束**：遵守 [`AGENTS.md`](../AGENTS.md) 单文件规模规范，新增子系统均拆分为独立子模块（目标 `<= 800` 行，硬性上限 `1500` 行），不继续堆砌进 `proxy.rs`、`ca.rs`、`present.rs` 或 `lib.rs`。
- **明确边界（Non-Goals）**：
  - **不做成重型 Postman / 团队 API 协作平台**：不引入多层级集合目录树、账号体系或云端同步。
  - **不做网卡级 TUN/TAP 透明代理与 HTTP/3 / QUIC 拦截**：保持显式 HTTP 代理模型。
  - **不内嵌重度 C/C++ 脚本运行时（如 Node / Python / C 版 QuickJS）**：避免破坏 macOS / Windows / Linux / `wasm32` / Docker 静态交叉编译的确定性与轻量二进制体积。
  - **不做高复杂度安全 Fuzzer / 漏洞扫描器**。

---

## 二、与主流同类产品的能力横评矩阵

对比当前主流抓包与 API 调试工具：**Proxyman**、**HTTP Toolkit**、**Reqable**、**mitmproxy**、**Charles**。

| 能力维度 | zniff (`v0.3.0`) | Proxyman | HTTP Toolkit | Reqable | mitmproxy | Charles |
|---|---|---|---|---|---|---|
| **架构与轻量度** | ✅ 原生 Rust + `nsh 0.6`（内存有界分片） | 原生 Swift（仅 macOS，Windows 另写） | Electron（体积大、内存高） | Flutter + C++ 引擎 | Python（Terminal/Web） | Java JVM（较笨重） |
| **同构 Web / 远程操作台** | ✅ `--web` 共用同份 Rust→Wasm 界面 + `--relay` | ⚠️ 无完整同构控制台 | ❌ 本地桌面为主 | ❌ 移动端协同 app | ✅ `mitmweb` | ❌ 无 |
| **AI 原生 / MCP** | ✅ 内置 `POST /mcp` 与 `--mcp`（10 个 Tools，含 `generate_openapi`） | ❌ 无原生 MCP | ❌ 无 | ⚠️ 有限辅助 | ⚠️ 需外部脚本 | ❌ 无 |
| **隐私脱敏与静态分享** | ✅ HAR/HTML/实时分享 + 结构化 `multipart` + `redact_strict` | ❌ 无结构化严格遮罩 | ❌ 仅本地导出 | ❌ 仅基础导出 | ❌ 需脚本手写 | ❌ 无 |
| **纯文本可分享规则** | ✅ 16 类快捷规则 + OR / `op=` / `#` 单行启停 / Chunked 改写 | UI 规则列表（JSON 导入导出） | UI 规则匹配器 | 重写规则 + Python 脚本 | Python 脚本 | UI Rewrite / Map |
| **零配置单终端/单浏览器隔离** | ✅ **独立 SPKI Chromium + 预注入代理终端 + `adb` 一键反代** | ✅ 独立终端/iOS/Android 脚本 | ✅ **极佳（独立浏览器/终端/adb）** | ✅ 协同手机端 + 脚本 | ⚠️ 手配环境变量 | ❌ 主要改全局代理 |
| **动态签名 / 自定义加解密** | ✅ **内置 `{{hmac_sha256}}` 模板变量 + 本地 HTTP `hook` 扩展** | ✅ JS 脚本钩子 | ⚠️ 收费脚本步骤 | ✅ Python 脚本 | ✅ **极强 Python 插件** | ❌ 弱 |
| **Schema 级协议解码** | ✅ **支持 `.proto` Schema 挂载解码（零 `protoc` 依赖）+ JSON/MsgPack 树形视图** | ✅ 支持 `.proto` 解析 | ✅ 支持协议拓展 | ⚠️ 基础预览 | ✅ Protobuf 插件 | ✅ 支持 `.proto` 映射 |
| **从抓包沉淀 Mock / OpenAPI** | ✅ **一键生成本地 Mock 目录（`fallback`）+ 导出 OpenAPI 3.0.3** | ✅ 导出 OpenAPI / Mock | ✅ 从流量建 Mock | ✅ API 集合 + Mock | ❌ 需插件 | ❌ 仅单条 Map Local |

### 小结：优势与当前进展

1. **已建立或补齐的核心能力**：
   - 单二进制跨平台轻量体验、环形分片内存治理、同构 `--web` / `--relay` 分享、企业级结构化脱敏预览、内置 10 个 MCP Tools（含 `generate_openapi`）与纯文本可分享规则体系。
   - **已落地零配置接入三件套（`launcher.rs`）**：无需修改系统代理或钥匙串即可拉起独立 Chromium / 预注入终端 / Android `adb`。
   - **已落地两层零 C 动态扩展（`rule_vars.rs` + `webhook.rs`）**：支持 `{{hmac_sha256}}` 等动态模板签名及本机回环 HTTP `hook` 改写。
   - **已落地抓包资产导出（`openapi.rs` + `mock_gen.rs`）**：支持一键导出 OpenAPI 3.0.3 与批量生成带 `fallback` 规则的本地 Mock 目录。
2. **下一阶段剩余差距**：
   - **`.proto` Schema 真实字段解码**（对标 Charles / Proxyman / HTTP Toolkit）——当前 Protobuf/gRPC 仍显示 `field #N` Wire 标签，待 Phase 3 下一步挂载 `.proto` 描述符解码与可选的 `hook mode=preview` 只读解密视图。

---

## 三、三大演进方向与技术架构设计

### 方向一（Phase 1）：极致「零配置接入」三件套（Zero-Config Interception） — ✅ 已落地

核心目标：**不动全局系统代理、不污染系统全局证书库**，一键打开即抓纯净流量。

- **新增模块**：`app/src/launcher.rs`（仅非 wasm32 目标 `#[cfg(not(target_arch = "wasm32"))]`），前端入口收敛于顶栏「更多」`快速抓包 [浏览器][终端][ADB]`（[`app/src/ui/more_menu_rows.rs`](../app/src/ui/more_menu_rows.rs)）。

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

在保留纯文本 DSL 的前提下，让 `header`、`query`、`json`、`replace`、`map-local` 参数值在代理转发时支持 `{{...}}` 动态求值：

- **基础与随机变量**：`{{uuid}}`、`{{ts}}`（秒级时间戳）、`{{ts_ms}}`（毫秒级时间戳）、`{{iso_now}}`、`{{random_int(min,max)}}`
- **当前请求上下文引用**：`{{req.method}}`、`{{req.host}}`、`{{req.path}}`、`{{req.query.<key>}}`、`{{req.header.<name>}}`、`{{req.body}}`
- **纯 Rust 摘要与编码（零额外依赖实现 RFC 6234 SHA-256 与 RFC 2104 HMAC-SHA256）**：
  - `{{sha256(x)}}`、`{{hmac_sha256(secret, msg)}}`、`{{hmac_sha256_b64(secret, msg)}}`
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
  - `mode=mutate`（默认）：代理将 `<= 1 MB` 有界请求/响应 JSON `{id, phase, method, url, status, headers, body_b64}` POST 到本地 hook，按返回的 `{status?, headers?, body_b64?}` 原地替换后再转发。
  - `mode=preview`：完全不改动转发字节，仅在详情页「美化 / 树形」视图非阻塞调用本地 hook 解密渲染并缓存结果，切回「原文」可对照原始密文。
- **安全与稳定性护栏**：
  - 默认仅允许向本机回环地址（`127.0.0.1` / `localhost` / `[::1]`）发起 hook 请求，防止配置误写导致流量外泄；
  - hook 客户端请求直连且不进入 `zniff` 自身代理与会话列表，杜绝自环死锁；
  - 统一受限于 `timeout` 与 `1 MB` `BODY_CAP`，超时按约定在备注记录警告后继续放行。

---

### 方向三（Phase 3 / v0.6）：`.proto` Schema 解码与一键生成 Mock / OpenAPI

#### 3.1 `.proto` Schema 挂载解码（`app/src/proto_schema.rs`） — ✅ 已落地

- **选型方案**：采用纯 Rust 描述符解析生态（如 `protox` + `prost-reflect`），**无需本机安装 `protoc` 命令行、零 C 依赖**，Wasm/全平台均可编译。
- **规则与挂载方式**：
  ```text
  proto api.vicanso.dev/grpc ./protos [req=<pkg.ReqMsg>] [res=<pkg.ResMsg>]
  ```
- **gRPC 路径自动推断**：对标准的 gRPC `:path` `/package.Service/Method`，自动从加载的 `.proto` `FileDescriptorSet` 查找对应 `MethodDescriptor` 的输入与输出 `MessageDescriptor`，将二进制 Wire 载荷解码为带真实字段名、枚举名称与嵌套结构的 JSON。
- **UI 无缝衔接**：复用 [`app/src/ui/detail_body.rs`](../app/src/ui/detail_body.rs) 现有的「`美化 | 树形 | 原文`」切换条——配置好 `.proto` 后自动从通用 `field #N` Wire 视图升级为完整语义 JSON 与可折叠树形视图。

#### 3.2 一键生成本地 Mock 目录与 OpenAPI 3.0.3 导出（`app/src/mock_gen.rs` & `app/src/openapi.rs`） — ✅ 已落地

- **一键生成本地 Mock 目录（Batch Map Local）**：
  - 支持对「当前筛选结果」批量落盘为 `<output_dir>/<host>/<path>[.<METHOD>].json` 静态目录结构（自动遵守当前 `redact_export` / `redact_strict` / `redact_fields` 脱敏设定），并在 `rules::resolve_map_local_target` 中扩展 `index.json` 与 `<seg>.json` 回退解析；
  - 生成完成后一键追加对应 `map-local <host> "<output_dir>" fallback` 规则，让前后端联调从「依赖不稳定的测试环境」秒切为「可离线复现的本地 Mock 套件」。
- **一键导出 OpenAPI 3.0.3 JSON**：
  - 按 `host + method + normalized_path` 自动聚类（自动识别纯数字 ID、UUID 路径段合并为 `{id}` 参数）；
  - 合并多次请求的 Query 参数、Header、Request JSON Schema 与按状态码分类的 Response JSON Schema（自动推断多样本可选字段）；
  - 已在顶栏「更多 -> 导出 `[HAR][HTML][OpenAPI]`」、Web API `GET /api/export.openapi.json` 及第 10 个内置 MCP Tool `generate_openapi` 中落地，让 AI 编码助手直接拿到准确的真实线上接口契约。

---

## 四、分阶段实施优先级与验收状态

| 阶段 | 核心交付 | 模块 | 状态 | 跨平台 / Wasm 边界 | 回归验收情况（参照 `zniff-regression`） |
|---|---|---|---|---|---|
| **Phase 1** | **零配置接入三件套**：免证书独立 Chromium、预注入代理终端、`adb` 一键代理 | `app/src/launcher.rs`（联动 `ca.rs`、`present.rs`、`more_menu_rows.rs`） | ✅ **已落地** | 仅桌面原生构建开放；Wasm UI 自动隐藏外部进程拉起入口 | Tier 1 全通过 + SPKI/env/adb 单测覆盖 |
| **Phase 2** | **两层轻量动态扩展**：`{{hmac_sha256}}` 等内置变量 + `hook` 本地 HTTP 钩子 | `app/src/rule_vars.rs`、`app/src/webhook.rs`（联动 `rules.rs`、`proxy.rs`） | ✅ **已落地** | 纯 Rust 规则求值全平台可用；`webhook` 限本机回环调用 | 模板求值单测 + `smoke-proxy.sh` 动态签名与 `/__hook__` 实测通过 |
| **Phase 3a** | **抓包资产导出**：批量 Mock 目录导出（含 `.json`/`index.json` 回退）、OpenAPI 3.0.3 + 第 10 个 MCP Tool `generate_openapi` | `app/src/mock_gen.rs`、`app/src/openapi.rs`（联动 `rules.rs`、`mcp.rs`、`web.rs`、`more_menu.rs`） | ✅ **已落地** | OpenAPI 生成兼容桌面与 Wasm；Mock 批量落盘限于桌面端 | 路径归并 `{id}` / Schema 合并 / Mock 导出单测 + `smoke-proxy.sh` `/api/export.openapi.json` & `generate_openapi` 实测 + Tier 1–3 验收 |
| **Phase 3b** | **`.proto` Schema 挂载解码** | `app/src/proto_schema.rs`（联动 `rules.rs`、`proxy.rs`、`detail_publish.rs`） | ✅ **已落地** | 纯 Rust 零外部依赖解析 `.proto` 与 gRPC 帧，无需安装 `protoc` | 嵌套消息/枚举/`map`/packed/gRPC RPC 单测 + Tier 1–2 全量回归通过 |
