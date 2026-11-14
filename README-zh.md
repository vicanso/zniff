# zniff

<p align="center">
  <strong>基于纯 Rust 的单进程 HTTP(S) 抓包调试代理 — 集成原生 GUI、WebAssembly 操作台与内置 MCP 服务。</strong>
</p>

<p align="center">
  <a href="./README.md">English</a> | 简体中文
</p>

<p align="center">
  <a href="https://github.com/vicanso/zniff/releases"><img src="https://img.shields.io/badge/version-v0.2.0-blue.svg" alt="Version"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-2024%20edition-orange.svg" alt="Rust 2024"></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-green.svg" alt="License"></a>
  <a href="#快速上手"><img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows%20%7C%20Docker-lightgrey.svg" alt="Platform"></a>
  <a href="#运行模式与-mcp"><img src="https://img.shields.io/badge/MCP-JSON--RPC%202.0-blueviolet.svg" alt="MCP Ready"></a>
</p>

https://github.com/user-attachments/assets/15117c1d-50a0-4578-8201-a305fb0d12a3

---

## 核心特性

- **原生轻量**：代理内核与 `nsh` GUI 编译为单个纯 Rust 二进制程序，启动迅速且内存占用低。
- **严格协议跟随**：解析 `ClientHello` 的 ALPN 提案并直接与源站协商。支持 HTTP/2 多路复用、WebSocket（RFC 6455）与 SSE，不降级为 HTTP/1.1。
- **深度检视**：支持 JSON（含可折叠、可搜索的 JSON 树形视图）、GraphQL（自动拆分 `operationName`、查询语句与变量）、HTML/XML、无模式 Protobuf/gRPC 与 MessagePack 解码；行内自动解码 JWT、Unix 时间戳和 Base64，支持基准行级 Diff、正文检索与连接阶段耗时拆分（DNS / TCP / TLS、连接复用）。
- **14 项改写规则与断点队列**：包含终点规则（`map-local`（支持目录映射与回源）、`map-remote`、`break`、`break-res`、`block`）与叠加修饰规则（`replace`、`json`、`delay`、`throttle`、`header`、`query`、`cors`、`no-cache`、`cert` / `client-cert`），支持 GraphQL `op=` 精准匹配与 8 槽位 FIFO 断点排队。
- **快速定位**：丰富的过滤语法（`host:`、`path:`、`op:`、`body:`、`header:`、反向 `-host:` / `-status:` 等）、域名与应用分面抽屉、列表二次搜索、列拖拽排序，以及带历史快照的命名配置。
- **图表、导出与协作**：一键生成瀑布图、泳道时序图、依赖流转图（`flowchart TD`）及 Mermaid 源码；可复制为 cURL、`fetch`、Python `requests`、Rust `reqwest`、Go `net/http`、TypeScript 类型或 Rust `serde` 结构体；支持导出 HAR/HTML（可选脱敏）与 `/s/{uuid}` 实时分享、`--relay` 中转。
- **AI 原生 MCP 与网页控制台**：内置 JSON-RPC 2.0 MCP 服务（`POST /mcp` 与 `zniff --mcp`），以及无界面 WebAssembly 控制台（`zniff --web`），控制台与抓包端口均可开启 HTTP Basic 认证。
- **自动更新**：自动检查 GitHub / Gitee 发布并原地安装更新。

---

## 界面预览

| 深度检视（GraphQL 与 JSON） | 过滤与 14 项改写规则 |
| :---: | :---: |
| ![深度检视](./docs/screenshots/02_detail_json_graphql.png) | ![过滤与规则](./docs/screenshots/05_filter_rules_popup.png) |
| **瀑布图 / 时序图与断点栏** | **自适应浅色与深色主题** |
| ![瀑布图与时序图](./docs/screenshots/07_sequence_diagram_waterfall.png) | ![自适应浅色主题](./docs/screenshots/10_theme_light_contrast.png) |

---

## 快速上手

### 1. 安装

**macOS、Linux 与 Windows**
从 [GitHub Releases](https://github.com/vicanso/zniff/releases) 下载预编译安装包：macOS 为已签名的 `.dmg`（Apple Silicon / Intel），Linux（x86_64 / aarch64）与 Windows（x86_64，内含 `zniff.exe`）为 `.tar.gz`。Linux 解压后可运行包内的 `install-linux.sh` 做用户级安装（启动器图标与开机自启；支持 `--no-autostart`、`--uninstall`、`--tag vX.Y.Z`）。也可通过 Cargo 从源码安装：
```bash
cargo install --git https://github.com/vicanso/zniff
# 或克隆仓库后打包 macOS .app：
# git clone https://github.com/vicanso/zniff.git && cd zniff && make bundle
```

**Docker（Web 控制台）**
```bash
docker run --rm -p 8080:8080 -p 8877:8877 \
  -v "$PWD/zniff-data:/data" \
  vicanso/zniff-web:latest
```
访问 `http://127.0.0.1:8080`，点击「开始」后开启抓包端口 `8877`。镜像同时暴露中继端口 `8879`（配合 `--relay --token <密钥>` 使用）。需要最新开发版可使用 `:nightly` 标签。

### 2. 配置客户端与根证书

1. 启动 `zniff`（默认监听 `127.0.0.1:8877`）。
2. 在顶栏 **更多 → 复制代理环境** 获取终端环境变量：
   ```bash
   export HTTP_PROXY="http://127.0.0.1:8877"
   export HTTPS_PROXY="http://127.0.0.1:8877"
   export ALL_PROXY="http://127.0.0.1:8877"
   export SSL_CERT_FILE="$HOME/Library/Application Support/zniff/ca.crt"
   ```
3. **信任根证书**：
   - **macOS**：**更多 → 证书 → 安装**，自动导入并信任到系统钥匙串。
   - **iOS 模拟器**：**更多 → 安装证书到 iOS 模拟器**，通过 `xcrun simctl` 为所有已开机模拟器配置证书和代理。
   - **移动真机**：点击底栏「扫码」图标，或手机访问 `http://<局域网IP>:8877/zniff`。

---

## 改写规则

每行一条规则，多个域名或路径可用 `|` 或 `,` 连接：

```text
# 终点规则（第一条命中生效）
map-local  api.example.com/v1/user ./mocks/user.json
map-remote api.example.com/api http://127.0.0.1:3000/api
break      api.example.com/v1/checkout
block      ads.example.com status=403

# 叠加修饰规则（按序累加）
json       api.example.com/v1 res set /data/is_vip true
delay      api.example.com/graphql 2s op=Update*
header     *.example.com res del Content-Security-Policy
```

完整语法与示例请参阅 **[规则与配置指南](./docs/rules-zh.md)**（[English Guide](./docs/rules.md)）。

---

## 运行模式与 MCP

| 模式 | 命令 | 说明 |
| :--- | :--- | :--- |
| **桌面 GUI** | `zniff` | 启动原生桌面窗口与进程内代理内核。 |
| **Web 控制台** | `zniff --web` | 启动 HTTP 控制台（默认 `127.0.0.1:8080`），点击开始后监听 `8877`。 |
| **Web + 中继** | `zniff --web --relay --token <密钥>` | 同一进程内同时提供 Web 控制台与中继（默认 `0.0.0.0:8879`；加 `--no-capture` 只做中继）。 |
| **独立中继** | `zniff --relay --token <密钥>` | 独立运行的中继服务，用于 `/s/{uuid}` 实时分享。 |
| **MCP 桥接** | `zniff --mcp` | 标准输入输出 MCP 桥接（同时提供本地回环 `POST /mcp`）。 |

全局参数：`--data-dir <目录>`（环境变量 `ZNIFF_DATA_DIR`）覆盖配置与证书目录；`--proxy-auth <user:pass>` 为抓包端口开启认证；`--web-prefix` / `--relay-prefix` 用于挂在反向代理子路径下；`zniff --help` 查看全部参数，`zniff --version` 输出版本号。

<details>
<summary>Claude Desktop / Cursor MCP 配置与认证参数</summary>

```json
{
  "mcpServers": {
    "zniff": {
      "command": "zniff",
      "args": ["--mcp", "--port", "8877"]
    }
  }
}
```

内置 MCP 工具：`list_flows`、`get_flow_detail`、`get_session_summary`、`annotate_flow`、`replay_flow`、`get_rules`、`set_rules`。

| 端口 | 认证方式 | 参数 / 环境变量 |
| :--- | :--- | :--- |
| 控制台（`8080`） | HTTP Basic | `--web-auth <user:pass>` / `ZNIFF_WEB_AUTH` |
| 抓包端口（`8877`） | `Proxy-Authorization` (407) | `--proxy-auth <user:pass>` / `ZNIFF_PROXY_AUTH` |
| 中继上传 | `Authorization: Bearer` | `--token <secret>` / `relay_token` |
| 中继查看 | HTTP Basic | `--view-auth <user:pass>` / `ZNIFF_RELAY_VIEW_AUTH` |

</details>

---

## 快捷键、数据目录与开发

| 快捷键 | 功能 |
| :--- | :--- |
| `Cmd/Ctrl + F` | 聚焦过滤输入框 |
| `Cmd/Ctrl + K` | 清空当前会话 |
| `Cmd/Ctrl + R` | 重放选中流量 |
| `Cmd/Ctrl + Enter` | 发送构造请求 / 放行当前断点 |
| `↑` / `↓` | 上下切换选中行 |
| `Esc` | 收起弹窗或返回列表统计视图 |

**数据目录**：
- **macOS**：`~/Library/Application Support/zniff`
- **Linux**：`$XDG_DATA_HOME/zniff` 或 `~/.local/share/zniff`
- **Windows**：`%APPDATA%\zniff`

可用 `--data-dir <目录>` 或 `ZNIFF_DATA_DIR` 覆盖（Docker 镜像使用 `/data`）。包含 `config.toml`、`ca.crt` / `ca.key`、`proxy-snapshot.toml` 与开发模式配置 `config.dev.toml`（默认端口 `8878`）。会话内存上限 256 MB，未星标且超过 16 KB 的正文自动转存至最大 512 MB 的环形临时分片（`zniff-spill/`）。

**本地构建与测试**：
```bash
make dev      # 启动桌面开发模式（端口 8878）
make release  # 构建 Release 二进制
make bundle   # 打包 macOS zniff.app
make web      # 启动 Web 控制台（端口 8080）
make test     # 运行测试
make lint     # 运行 clippy
```

---

## 开源许可证

[Apache License, Version 2.0](./LICENSE)
