# zniff 使用指南

[English](./guide.md) | 简体中文

本文介绍 zniff 的完整能力、客户端接入方式，以及如何从 Charles / Whistle 迁移。完整规则语法见 [规则与配置指南](./rules-zh.md)；无界面运行模式见 [命令行、Web 控制台与 MCP](./cli-web-mcp-zh.md)。

## 目录

- [核心特性](#核心特性)
- [安装](#安装)
- [客户端与根证书配置](#客户端与根证书配置)
- [从 Charles / Whistle 迁移](#从-charles--whistle-迁移)
- [改写规则速览](#改写规则速览)
- [快捷键](#快捷键)
- [数据目录与内存](#数据目录与内存)
- [本地构建与测试](#本地构建与测试)

## 核心特性

- **原生轻量与免配抓包**：代理内核与 `nsh 0.6.1` GUI 编译为单个纯 Rust 二进制（内存有界、正文自动落盘）。通过底栏 **「接入」** 弹窗或引导卡一键拉起 **隔离 Chromium 浏览器**（SPKI 指纹信任，不动钥匙串）、**代理终端**、**Android `adb` 端口反代**、**iOS 模拟器** 或手机扫码；空会话引导卡还支持一键 **「加载示例流量」**，不配代理也能立即体验。
- **出错可自救与低摩擦日常体验**：失败（`ERR` / `407`）流量在详情摘要直接给出修复按钮（**加入直通**、**安装证书**、**隔离浏览器打开**、**检查上游**、**添加 resolve**、**打开安全设置**）；过滤结果为空时自动诊断是哪个过滤词导致无匹配并提供一键清空/暂停；`Cmd/Ctrl+K` 清空支持 **5 秒撤销 Toast**；按 `?` 或 `Cmd/Ctrl+/` 查看快捷键速查，`Cmd/Ctrl+Shift+P` 打开全局命令面板。
- **严格协议跟随与 SOCKS5 上游**：解析 `ClientHello` ALPN 提案并直接与源站协商，原生支持 HTTP/2 多路复用、WebSocket（RFC 6455）与 SSE，不降级 HTTP/1.1；上游代理支持 HTTP/CONNECT、`socks5://` 与 `socks5h://`。
- **深度检视、AI SSE 聚合与 Schema 解码**：美化 JSON + 可搜索 JSON 树形视图、GraphQL（拆分 `operationName`/查询/变量）、**LLM / AI SSE 流式聚合**（OpenAI / Anthropic / Gemini 汇总 Thinking、正文、`tool_calls`、TTFT 与 Token/s）、HTML/Markdown 预览、XML、MessagePack、零依赖 `.proto` Schema 解码、行内 JWT/时间戳/Base64 注解、基准行级 Diff 与连接耗时拆分。
- **17 类规则与多组叠加**：支持独立开关的多规则组（`[[rule_groups]]`），涵盖终点规则（`map-local`、`map-remote`、`break`、`break-res`、`block`）、叠加修饰（`replace`、`json`、`delay`、`throttle`、`header`、`query`、`cors`、`no-cache`）、域名级控制（`cert` / `client-cert`、`resolve`）与扩展规则（本地 `hook` 改写/只读解密、`proto`），支持 `{{uuid}}` / `{{hmac_sha256(...)}}` 动态变量、GraphQL `op=` 精准匹配与 8 槽位 FIFO 断点队列。
- **图表、OpenAPI、Mock 与协作**：瀑布图、泳道时序图、依赖流转图（`flowchart TD`）与 Mermaid 源码；复制为 cURL、`fetch`、Python、Rust、Go、TypeScript 类型或 Rust `serde` 结构体；导出 OpenAPI 3.0.3、批量本地 Mock 目录、脱敏 HAR/HTML，以及 `/s/{uuid}` 与 `--relay` 实时分享。
- **AI 原生 MCP 与网页控制台**：内置 JSON-RPC 2.0 MCP 服务（`POST /mcp` 与 `zniff --mcp`，10 个工具）及 WebAssembly 控制台（`zniff --web`），支持 Basic 认证与抓包端口域名准入控制（`allow_hosts`，Docker 默认严格全拒），桌面端支持 `SHA256SUMS` 校验原地更新。详见 [命令行、Web 控制台与 MCP](./cli-web-mcp-zh.md)。

过滤语法（`host:`、`path:`、`status:`、`body:`、`op:`、`-词` 等）见 [规则与配置指南 → 流量过滤语法](./rules-zh.md#2-流量过滤语法traffic-filter)。

## 安装

从 [GitHub Releases](https://github.com/vicanso/zniff/releases) 下载预编译安装包：macOS 为已签名的 `.dmg`（Apple Silicon / Intel），Linux（x86_64 / aarch64）与 Windows（x86_64 / aarch64，内含 `zniff.exe`）为 `.tar.gz`。Linux 解压后可运行包内的 `install-linux.sh` 做用户级安装（启动器图标与开机自启；支持 `--no-autostart`、`--uninstall`、`--tag vX.Y.Z`）。**Homebrew Cask**、**Scoop** 与 **Winget** 清单模板位于 [`packaging/`](../packaging/README.md)。也可通过 Cargo 从源码安装：

```bash
cargo install --git https://github.com/vicanso/zniff
# 或克隆仓库后打包 macOS .app：
# git clone https://github.com/vicanso/zniff.git && cd zniff && make bundle
```

Docker Web 控制台见 [命令行、Web 控制台与 MCP → Docker](./cli-web-mcp-zh.md#docker)。

## 客户端与根证书配置

1. 启动 `zniff`（默认监听 `127.0.0.1:8877`）。
2. **零配置抓包（推荐）**：点击无流量引导卡、底栏 **「接入」** 弹窗或顶栏 **更多 → 免配抓包**，即可一键拉起 `[浏览器]`（免装系统证书的隔离 Chromium）、`[终端]`（预注入全套代理与 CA 环境变量）、`[ADB]`（Android USB 反代）或复制动态 `/proxy.pac` 自动代理地址。
3. **手动配置终端环境变量**：在顶栏 **更多 → 复制代理环境** 获取：
   ```bash
   export HTTP_PROXY="http://127.0.0.1:8877"
   export HTTPS_PROXY="http://127.0.0.1:8877"
   export ALL_PROXY="http://127.0.0.1:8877"
   export SSL_CERT_FILE="$HOME/Library/Application Support/zniff/ca.crt"
   ```
4. **信任根证书（如需抓系统全局或移动端流量）**：
   - **macOS**：**更多 → 证书 → 安装**，自动导入并信任到系统钥匙串。
   - **复用外部根证书（可选）**：自动探测本机已信任的 **Whistle**、**mitmproxy**、**mkcert** 或 **Proxyman** 根证书，可在 **更多 → 证书** 或底栏 **「接入」** 弹窗中一键切换复用（`external_ca`）。
   - **iOS 模拟器**：**更多 → 安装证书到 iOS 模拟器**，通过 `xcrun simctl` 为所有已开机模拟器配置证书和代理。
   - **移动真机**：点击底栏「扫码」图标，或手机访问 `http://<局域网IP>:8877/zniff`。

## 从 Charles / Whistle 迁移

如果你已经在用 Charles 或 Whistle，无需重新配置即可平滑切换：

1. **直接复用已有根证书（免重装）**：打开底栏 **「接入」** 或顶栏 **更多 → 证书**，zniff 会自动探测本机已信任的 **Whistle**、**mitmproxy**、**mkcert**、**Proxyman** 根证书，一键复用即可解密 HTTPS。
2. **拖拽或粘贴导入规则与会话**：直接把 **Whistle `.txt`**、**Charles `.xml`**（Map Local / Map Remote / Rewrite / Breakpoints）、**`.chlsj`** 会话或 **`.har`** 拖入窗口（或在 **过滤 → 导入** 中粘贴），自动转译为 zniff 规则，未映射项保留为 `#` 注释。
3. **零配置立即体验**：还没改代理设置？点击空会话引导卡的 **「加载示例流量」** 即可体验全部检视与图表功能，或从 **「接入」** 启动 **隔离浏览器** / **代理终端**。

| Charles / Whistle | zniff 对应写法 |
| :--- | :--- |
| Whistle `api.example.com file:///mocks/a.json` / Charles Map Local | `map-local api.example.com ./mocks/a.json` |
| Whistle `api.example.com http://127.0.0.1:3000` / Charles Map Remote | `map-remote api.example.com http://127.0.0.1:3000` |
| Whistle `api.example.com 127.0.0.1:8443` / DNS Spoofing | `resolve api.example.com 127.0.0.1:8443` |
| Whistle `resDelay://1500` / Charles 限速 | `delay api.example.com 1500ms` / `throttle api.example.com 256kb/s` |
| Charles Breakpoints | `break api.example.com/checkout` / `break-res api.example.com/checkout` |
| Charles SSL Proxying 排除域名 | 加入 **直通名单**（或在任意 TLS `ERR` 流量详情点击 **加入直通**） |

## 改写规则速览

每行一条规则，多个域名或路径可用 `|` 或 `,` 连接：

```text
# 终点规则（第一条命中生效）
map-local  api.example.com/v1/user ./mocks/user.json
map-remote api.example.com/api http://127.0.0.1:3000/api
break      api.example.com/v1/checkout
block      ads.example.com status=403

# 叠加修饰与动态变量（按序累加）
json       api.example.com/v1 res set /data/is_vip true
delay      api.example.com/graphql 2s op=Update*
header     api.example.com/v1 req set X-Signature {{hmac_sha256("secret", req.body)}}

# 本地 Webhook 扩展与 .proto Schema 解码
hook       api.example.com/v1/cipher http://127.0.0.1:9001/decrypt res mode=preview
proto      grpc.example.com ./protos
```

完整语法与示例请参阅 **[规则与配置指南](./rules-zh.md)**。

## 快捷键

| 快捷键 | 功能 |
| :--- | :--- |
| `Cmd/Ctrl + Shift + P` | 打开 / 关闭全局命令面板 |
| `?` 或 `Cmd/Ctrl + /` | 打开快捷键速查弹窗 |
| `Cmd/Ctrl + F` | 聚焦过滤输入框 |
| `Cmd/Ctrl + K` | 清空当前会话（支持 5 秒撤销 Toast） |
| `Cmd/Ctrl + R` | 重放选中流量 |
| `Cmd/Ctrl + Enter` | 发送构造请求 / 放行当前断点 |
| `↑` / `↓` | 上下切换选中行 |
| `Esc` | 收起弹窗或返回列表统计视图 |

## 数据目录与内存

- **macOS**：`~/Library/Application Support/zniff`
- **Linux**：`$XDG_DATA_HOME/zniff` 或 `~/.local/share/zniff`
- **Windows**：`%APPDATA%\zniff`

可用 `--data-dir <目录>` 或 `ZNIFF_DATA_DIR` 覆盖（Docker 镜像使用 `/data`）。包含 `config.toml`、`ca.crt` / `ca.key`、`proxy-snapshot.toml` 与开发模式配置 `config.dev.toml`（默认端口 `8878`）。会话内存上限 256 MB，未星标且超过 16 KB 的正文自动转存至最大 512 MB 的环形临时分片（`zniff-spill/`）。

## 本地构建与测试

```bash
make dev      # 启动桌面开发模式（端口 8878）
make release  # 构建 Release 二进制
make bundle   # 打包 macOS zniff.app
make web      # 启动 Web 控制台（端口 8080）
make test     # 运行测试
make lint     # 运行 clippy
```
