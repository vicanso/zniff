# zniff

<p align="center">
  <strong>基于 Rust 构建的高性能单进程 HTTP(S) 抓包调试代理，集成本地原生 GUI、WebAssembly 网页操作台与 AI 原生 MCP 协议。</strong>
</p>

<p align="center">
  <a href="./README.md">English</a> | 简体中文
</p>

<p align="center">
  <a href="https://github.com/vicanso/zniff/releases"><img src="https://img.shields.io/badge/version-v0.1.0-blue.svg" alt="Version"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-2024%20edition-orange.svg" alt="Rust 2024"></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-green.svg" alt="License"></a>
  <a href="#快速上手"><img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows%20%7C%20Docker-lightgrey.svg" alt="Platform"></a>
  <a href="#为什么选择-zniff"><img src="https://img.shields.io/badge/architecture-single--process%20%7C%20zero--electron-purple.svg" alt="Architecture"></a>
  <a href="#9-ai-原生-mcp-协议集成"><img src="https://img.shields.io/badge/MCP-JSON--RPC%202.0-blueviolet.svg" alt="MCP Ready"></a>
</p>

---

<p align="center">
  <img src="./docs/screenshots/01_traffic_table_dual_pane.png" alt="zniff 双栏流量列表" width="100%">
</p>

---

## 目录

- [为什么选择 zniff？](#为什么选择-zniff)
- [核心特性对比横评](#核心特性对比横评)
- [快速上手](#快速上手)
- [核心功能图文详解](#核心功能图文详解)
  - [1. 双栏流量列表、列定制与分面导航](#1-双栏流量列表列定制与分面导航)
  - [2. 深度检视：JSON/GraphQL 格式化与正文行内检索](#2-深度检视jsongraphql-格式化与正文行内检索)
  - [3. 二进制解码器：无模式 Protobuf/gRPC 与 Hex 十六进制视图](#3-二进制解码器无模式-protobufgrpc-与-hex-十六进制视图)
  - [4. 基准对比与行级双栏 Diff](#4-基准对比与行级双栏-diff)
  - [5. 14 项快速改写规则与语法校验 DSL](#5-14-项快速改写规则与语法校验-dsl)
  - [6. 断点拦截与并发排队队列](#6-断点拦截与并发排队队列)
  - [7. 交互式 4 视图时序图与瀑布流图表](#7-交互式-4-视图时序图与瀑布流图表)
  - [8. 移动端抓包、扫码安装与 iOS 模拟器一键信任](#8-移动端抓包扫码安装与-ios-模拟器一键信任)
  - [9. AI 原生 MCP 协议集成](#9-ai-原生-mcp-协议集成)
  - [10. 自适应浅色与深色主题](#10-自适应浅色与深色主题)
- [规则 DSL 语法速查](#规则-dsl-语法速查)
- [命令行参数与运行模式](#命令行参数与运行模式)
- [常用快捷键](#常用快捷键)
- [配置持久化与数据目录](#配置持久化与数据目录)
- [本地开发与编译构建](#本地开发与编译构建)
- [开源许可证](#开源许可证)

---

## 为什么选择 zniff？

市面上的大多数 HTTP(S) 抓包工具要么是体积臃肿的 Electron 套壳应用，要么依赖笨重的 JVM 虚拟机环境，或是由多个高资源开销的后台服务拼装而成。**zniff** 采用极具革新性的架构：以 **纯 Rust** 编写超高吞吐的代理内核与轻灵的原生 GUI 界面，并编译为一个 **零外部依赖的自包含单一二进制程序**。

- ⚡ **极致原生性能**：冷启动仅需 **< 30 ms**，待机内存占用 **< 25 MB**。桌面版将代理内核与基于 `nsh` 的原生 GUI 融合在同一个进程内运行，彻底告别 Electron、告别 JVM、告别臃肿的 Web 运行时。
- 🤖 **AI 原生 Model Context Protocol (MCP)**：内置标准 JSON-RPC 2.0 规格的 MCP 服务，支持本地回环 HTTP `POST /mcp`（免代理认证）与标准输入输出桥 `zniff --mcp`。完美直连 **Claude Desktop**、**Cursor** 以及各类自主 AI Agent，实现自动化流量检索、详情审查、接口重放与规则热改写。
- 🌐 **严格协议跟随（零协议降级）**：实时解析客户端 TLS `ClientHello` 中的 ALPN 提案并直接与源站握手。客户端与源站之间严格采用源站协商的最高协议（HTTP/2 多路复用、RFC 6455 WebSocket 旁路帧嗅探、SSE 流式解析），杜绝降级至 HTTP/1.1，保留原始请求头大小写。
- 🛡️ **内存有界与大正文透明落盘**：严格限制内存会话上限为 256 MB。完成传输且未星标的大于 16 KB 请求/响应正文自动顺序转存至系统临时目录下的环形分片文件（`zniff-spill/`，8 个分片最大 512 MB）。Unix 环境下打开即 `unlink` 匿名化，进程退出由操作系统内核保证零泄漏回收。支持星标最多 32 条关键流量长驻内存豁免淘汰。
- 🎯 **14 种流量改写规则与 GraphQL `op=` 精准收敛**：内置 14 项快速规则，涵盖终点规则（`map-local` 本地映射、`map-remote` 远程重定向、`break` 请求断点、`break-res` 响应断点、`block` 阻断连接）与叠加修饰规则（`replace` 文本替换、`json` 指针改写、`delay` 延迟注入、`throttle` 弱网限速、`header` 头部增删、`query` 改参、`cors` 跨域补齐、`no-cache` 禁用缓存、`cert`/`mtls` 证书认证）。其中 11 种规则均支持 `op=<GraphQL操作名>` 通配匹配。
- 📊 **交互式 4 视图瀑布图与时序图分析**：将筛选后的网络流量一键转化为直观洞见：1) 瀑布图 Gantt 时间线（DNS、TCP、TLS、TTFB、下载耗时精准拆解），2) 泳道时序图，3) 依赖流转图（`flowchart TD`），4) Mermaid 源码查看与一键复制。

---

## 核心特性对比横评

| 维度 / 特性 | zniff | Proxyman | Charles | mitmproxy |
| :--- | :---: | :---: | :---: | :---: |
| **冷启动耗时** | **< 30 ms** | ~1–2 秒 | ~5–10 秒 | ~1 秒 |
| **空闲内存占用** | **~25–50 MB** | ~150–300 MB | ~500 MB–1.2 GB | ~100–200 MB |
| **客户端架构** | **单进程 Rust 原生程序** | 原生 Swift (macOS) / Electron (Win) | Java JVM 虚拟机 | Python 脚本运行时 |
| **HTTP/2 多路复用** | **ALPN 双向协议跟随** | 支持 | 解复用 / 降级 H1 | 支持 |
| **内置 AI MCP 接口** | **原生集成 (`POST /mcp` 与 stdio)** | ❌ 无 | ❌ 无 | ❌ 仅限社区第三方插件 |
| **WebAssembly 操作台** | **原生自带 (`zniff --web`)** | ❌ 无 | ❌ 无 | Python Web UI |
| **流量改写规则引擎** | **14 项规则 + GraphQL `op=`** | 脚本 / 本地映射 | 重写 / 映射 | Python Addons |
| **大体积正文保护** | **自动环形落盘分片 (>16KB)** | 专用缓存文件 | 磁盘缓存 | 流式内存阈值 |
| **开源协议与售价** | **完全开源 (Apache 2.0)** | 商业闭源 ($69–$99) | 商业闭源 ($50) | 完全开源 (MIT) |

---

## 快速上手

### 1. 安装与获取

#### macOS
```bash
# 通过预构建 .app 包或 Homebrew：
brew install zniff

# 或从源码快速构建：
git clone https://github.com/vicanso/zniff.git
cd zniff
make bundle   # 在 target/release/bundle/osx/ 生成 zniff.app
```

#### Linux 与 Windows
直接从 [GitHub Releases](https://github.com/vicanso/zniff/releases) 下载对应平台的独立预编译可执行文件，或通过 Cargo 本地编译安装：
```bash
cargo install --path app
```

#### Docker 容器（Web 控制台模式）
在无图形界面的服务器或 Docker 环境中运行 Web 操作台：
```bash
docker run --rm -p 8080:8080 -p 8877:8877 \
  -v "$PWD/zniff-data:/data" \
  vicanso/zniff-web:nightly
```
在浏览器中打开 `http://127.0.0.1:8080` 进入控制台，点击界面上的「开始」按钮即刻打开抓包监听端口 `8877`。

---

### 2. 客户端 5 秒快速配置

1. 打开 `zniff`，默认监听在 `127.0.0.1:8877`。
2. 在工具栏右侧「更多」菜单中，点击「复制代理环境」即可获得标准终端环境变量，粘贴进终端即可生效：
   ```bash
   export HTTP_PROXY="http://127.0.0.1:8877"
   export HTTPS_PROXY="http://127.0.0.1:8877"
   export ALL_PROXY="http://127.0.0.1:8877"
   export SSL_CERT_FILE="$HOME/Library/Application Support/zniff/ca.crt"
   ```
3. **HTTPS 解密（根证书信任）**：
   - **macOS**：在「更多」菜单中点击「证书：未安装」→「安装」，自动完成钥匙串导入与信任授权。
   - **iOS 模拟器**：在「更多」菜单点击「安装证书到 iOS 模拟器」，zniff 将通过 `xcrun simctl` 自动遍历所有已开机的模拟器注入信任证书并配置代理。
   - **移动真机**：点击底栏的「扫码」图标展示安装二维码，或使用手机浏览器直接访问 `http://<局域网IP>:8877/zniff` 查看指引并下载描述文件。

---

## 核心功能图文详解

以下所有界面截图均截取自真实运行中的 `zniff` 会话，保持统一的 `1920 x 1080` 全高清规格。

### 1. 双栏流量列表、列定制与分面导航

<p align="center">
  <img src="./docs/screenshots/01_traffic_table_dual_pane.png" alt="双栏流量列表" width="100%">
</p>

- **双栏弹性布局**：左侧列表支持在 280–1100 px 之间平滑拖拽分割条，并实时自适应窗口宽度保留右栏详情空间。列表左侧固定区锁定 `序号`、`方法`、`路径`，横向滚动时保持常驻可见。
- **10 列自由开关与排序**：支持 `序号`、`方法`、`路径`（可一键切换为完整 URL）、`状态`、`间隔`（距上一条流的相对时差，悬停查看绝对时间）、`耗时`（≥1s 高亮警告）、`大小`、`协议`、`主机`、`来源 IP`（自动异步识别本机回环进程名，如 `127.0.0.1 (curl)`）。
- **表头三态排序**：点击表头在升序（`↑`）、降序（`↓`）与到达顺序之间循环切换。
- **折叠式分面导航抽屉**：实时聚合当前会话中的域名（`host:`）与本地应用进程（`app:` / `proc:`）请求计数与错误数，点击胶囊快速组合筛选，点胶囊右侧 `×` 立即排除该域名。

---

### 2. 深度检视：JSON/GraphQL 格式化与正文行内检索

<p align="center">
  <img src="./docs/screenshots/02_detail_json_graphql.png" alt="JSON 与 GraphQL 深度检视" width="100%">
</p>

- **GraphQL 一等公民**：美化模式自动识别 GraphQL POST 请求，将 `operationName`、查询语句文本以及变量 JSON 拆分为独立区块高亮展示。
- **行内检索与命中高亮**：在请求与响应正文卡片标题栏内置跨行搜索框，输入关键字实时高亮并显示匹配命中数，支持行号精准跳转（`[L0001]`）。
- **智能解码提示**：在请求头、Query 查询参数以及表单字段中，自动识别并实时解码 **JWT** Token、10/13 位 Unix **时间戳**以及可打印 **Base64** 字符串。

---

### 3. 二进制解码器：无模式 Protobuf/gRPC 与 Hex 十六进制视图

<p align="center">
  <img src="./docs/screenshots/03_detail_protobuf_hex.png" alt="Protobuf 与 Hex 十六进制检视" width="100%">
</p>

- **无模式 Protobuf 与 gRPC Wire 解码**：无需提供任何 `.proto` 定义文件，zniff 能够直接解析二进制 Wire Tag（区分 Varint、64-bit、Length-delimited 子消息与 UTF-8 字符串），并自动剥离 gRPC 5 字节帧前缀。
- **MessagePack 自动转译**：自动将 MessagePack 二进制载荷解码为易读的 JSON 树结构。
- **自适应 Hex 视图**：支持 16 字节或 32 字节自适应排布，左侧标注 4 位十六进制字节偏移行号，右侧提供对应 ASCII 字符对照。

---

### 4. 基准对比与行级双栏 Diff

<p align="center">
  <img src="./docs/screenshots/04_baseline_diff_view.png" alt="基准对比 Diff 视图" width="100%">
</p>

- **一键设置基准流**：右键任意流量选择「设为基准」（列表前部以醒目的红色竖线标识）。
- **双栏 Diff 比较器**：点击选中另一条对比流量（以绿色竖线标识），右侧卡片将自动通过 `diff_view` 展开 URL、请求头、请求正文、响应头与响应正文的双栏行级差异对比。

---

### 5. 14 项快速改写规则与语法校验 DSL

<p align="center">
  <img src="./docs/screenshots/05_filter_rules_popup.png" alt="14 项快速改写规则" width="100%">
</p>

- **双栏配置弹出层**：点击顶栏「过滤」按钮呼出常驻弹出层，左侧为结构化过滤表达式与快捷勾选，右侧为 14 项规则的快速添加选项卡与规则编辑器。
- **实时语法校验**：输入规则时逐行校验语法正确性，实时提示错误行号，避免错误语法生效。
- **配置预设与快照回滚**：支持保存最多 16 套命名配置（`SavedProfile`），支持以 `.toml` 格式复制到剪贴板或通过拖拽 `.toml` 文件一键导入，同时自动保留最近 10 次修改的历史快照撤销栈。

---

### 6. 断点拦截与并发排队队列

<p align="center">
  <img src="./docs/screenshots/06_breakpoint_intercept_queue.png" alt="断点拦截排队队列" width="100%">
</p>

- **有界 FIFO 队列（8 个插槽）**：防止客户端并发请求堆积导致代理死锁，第 9 条到达的断点请求将自动放行队首最旧请求。
- **胶囊切换交互**：顶栏黄色断点条上显示当前并发排队胶囊（如 `#12 (1/3)`），点击胶囊即可瞬间切换到该请求进行检视与编辑。
- **原地修改与放行**：支持在放行前修改方法、URL、请求头、状态码与正文；支持「放行」、「全部放行」与「丢弃」（断开连接，记录为 `ERR · 断点丢弃`）。输入编辑期间超时时间从 60 秒自动延长至 5 分钟。

---

### 7. 交互式 4 视图时序图与瀑布流图表

<p align="center">
  <img src="./docs/screenshots/07_sequence_diagram_waterfall.png" alt="时序图与瀑布流图表" width="100%">
</p>

- **4 种交互式可视化视角**：
  1. **瀑布图（Waterfall）**：精确拆解每条流在 DNS 解析、TCP 建连、TLS 握手、TTFB 服务端首包等待与正文下载各阶段的微秒级耗时。
  2. **泳道时序图（Swimlane）**：展示客户端与多个后端微服务之间的时间轴交互，附带状态码与耗时标注。
  3. **依赖流转图（Dependency Flowchart）**：基于 `flowchart TD` 的请求上下游拓扑图。
  4. **Mermaid 源码页**：支持直接查看生成的 Mermaid 脚本并一键复制 Mermaid 或 Markdown 格式文本。
- **虚拟滚动支持**：基于 `virtual_list` 优化，勾选数百条流量仍可流畅滚动浏览。

---

### 8. 移动端抓包、扫码安装与 iOS 模拟器一键信任

<p align="center">
  <img src="./docs/screenshots/08_mobile_qr_cert_console.png" alt="移动端抓包与 CA 信任" width="100%">
</p>

- **移动端配置二维码**：底栏一键呼出局域网连接二维码，手机相机扫码即可直达 `http://<局域网IP>:<端口>/zniff` 获取安装指引并一键下载根证书。
- **iOS 模拟器全自动配置**：通过 `xcrun simctl` 自动将 `ca.crt` 写入并信任到所有已启动的 iOS 模拟器中，并自动配置系统 HTTP 代理。
- **Linux 证书安装命令生成**：自动识别发行版（Debian/Ubuntu/CentOS/Fedora/Arch）并在剪贴板生成对应的 `trust anchor` 或 `update-ca-certificates` 命令。

---

### 9. AI 原生 MCP 协议集成

<p align="center">
  <img src="./docs/screenshots/09_mcp_ai_integration.png" alt="AI 原生 MCP 协议集成" width="100%">
</p>

- **无缝对接 AI 智能体**：内置符合标准 JSON-RPC 2.0 规格的 Model Context Protocol (MCP) 服务，支持本地回环 `POST /mcp` 与命令行 stdio 桥接 `zniff --mcp`。
- **5 大核心 MCP 工具**：
  - `list_flows`：按状态码、请求方法、域名或关键词检索流量列表。
  - `get_flow_detail`：获取指定流量的全部头、请求正文、响应正文与连接耗时。
  - `replay_flow`：重放特定历史流量并观察实时响应。
  - `get_rules`：获取当前生效的全部改写规则与直通名单。
  - `set_rules`：通过 AI 自动配置并热更新改写规则。
- **Claude Desktop 与 Cursor 配置示例**：
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

---

### 10. 自适应浅色与深色主题

<p align="center">
  <img src="./docs/screenshots/10_theme_light_contrast.png" alt="自适应浅色与深色主题" width="100%">
</p>

- **精细化色彩分层**：基于 `AdaptiveColor::rgb` 实现双模色彩映射，浅色与深色模式下均具备极高的对比度与清晰度。
- **语义化状态徽章**：HTTP 状态码（`2xx` 绿色、`3xx` 青色、`4xx`/`5xx` 红色）、请求方法胶囊与耗时警告色分明，拒绝单调刺眼的灰暗背景。

---

## 规则 DSL 语法速查

每行一条规则，第二个参数为 `<pattern>`（格式为 `<域名匹配>[/<路径前缀>]`，多个域名或路径可用 `|` 或 `,` 连接）：

> 完整配置说明与典型实战示例请参阅：**[规则与配置指南](./docs/rules-zh.md)**（[English Guide](./docs/rules.md)）

### 终点规则（第一条命中生效）
```text
# 本地映射：使用本地文件或目录直接响应（状态码默认 200）
map-local api.example.com/v1/user ./mocks/user.json
map-local api.example.com/v1/error status=503

# 远程转发：重定向至其他主机或端口
map-remote api.example.com/api http://127.0.0.1:3000/api

# 请求断点：暂停发往源站前的请求
break api.example.com/v1/checkout

# 响应断点：暂停返回客户端前的响应
break-res api.example.com/v1/orders

# 规则阻断：返回自定义状态码或直接切断底层连接
block ads.example.com status=403
block tracker.example.com drop
```

### 叠加修饰规则（全部命中项按序累加）
```text
# 文本替换：改写请求路径/正文或 UTF-8 响应正文
replace api.example.com/v1 res "env":"prod" -> "env":"dev"
replace api.example.com/v1 req ?debug=false -> ?debug=true

# JSON 改写：按 JSON Pointer (/a/b) 或点分路径 (a.b) 修改/删除字段
json api.example.com/v1 res set /data/is_vip true
json api.example.com/v1 res del /data/deprecated_field

# 延迟注入：注入响应延迟（上限 30 秒）
delay api.example.com/v1/feed 1500ms

# 弱网限速：预设模式 (2g, 3g, 4g) 或自定义上下行速率
throttle api.example.com 3g
throttle video.example.com down=500k up=100k

# 头部改写：增删改 HTTP 请求头或响应头
header api.example.com req set X-Debug-Session active
header *.example.com res del Content-Security-Policy

# 参数改写：自动增删改 URL Query 参数
query api.example.com/v1 set env staging
query *.example.com del utm_source

# 跨域补齐：补全 CORS 响应头，OPTIONS 预检直接返回 204
cors api.example.com

# 禁用缓存：剥离条件头并注入 Cache-Control: no-cache
no-cache api.example.com/v1

# 自定义证书：指定域名服务端证书或上游 mTLS 双向认证客户端证书
cert internal.corp.net /path/to/server.crt /path/to/server.key
cert secure.bank.com /path/to/client.crt /path/to/client.key
```

### GraphQL `op=` 条件收敛
11 种规则均支持通过 `op=<操作名>` 进行精准过滤（支持 `*` 通配与 `|` 逻辑或）：
```text
# 仅针对 'GetUserProfile' 操作应用本地 Mock 数据
map-local api.example.com/graphql ./mocks/profile.json op=GetUserProfile

# 对所有以 'Update' 开头的 GraphQL Mutation 操作注入 2 秒延迟
delay api.example.com/graphql 2s op=Update*

# 仅拦截 QueryOrders 或 QueryCheckout 操作断点
break api.example.com/graphql op=QueryOrders|QueryCheckout
```

---

## 命令行参数与运行模式

zniff 支持 4 种独立的工作运行模式：

```text
用法:
  zniff [--data-dir <dir>] [--proxy-auth <user:pass>]
  zniff --web [--web-bind <ip>] [--web-port <port>] [--web-auth <user:pass>]
              [--bind <ip>] [-p|--port <port>] [--data-dir <dir>] [--proxy-auth <user:pass>]
              [--relay --token <secret> [--relay-bind <ip>] [--relay-port <port>]
               [--view-auth <user:pass>] [--no-capture]]
  zniff --relay --token <secret> [--view-auth <user:pass>] [--bind <ip>] [-p|--port <port>]
              [--data-dir <dir>]
  zniff --mcp [-p|--port <port>] [--data-dir <dir>]
```

| 运行模式 | 启动命令 | 功能说明 |
| :--- | :--- | :--- |
| **桌面 GUI 模式** | `zniff` | 启动原生桌面窗口并运行进程内代理内核。 |
| **Web 控制台模式** | `zniff --web` | 启动无界面 HTTP 控制台（默认 `8080`），点击「开始」后开启抓包端口 `8877`。 |
| **独立中继模式** | `zniff --relay --token <密钥>` | 独立运行的中继服务，用于托管与分发 `/s/{uuid}` 实时共享流量。 |
| **AI MCP 桥接模式** | `zniff --mcp` | 启动基于标准输入输出的 MCP 协议桥，直连运行中的抓包实例。 |

### 端口与身份认证凭据

| 监听端点 | 校验方式 | 配置参数 / 环境变量 |
| :--- | :--- | :--- |
| **控制台（`8080`）** | HTTP Basic | `--web-auth <user:pass>` / `ZNIFF_WEB_AUTH` |
| **抓包端口（`8877`）** | `Proxy-Authorization` (407) | `--proxy-auth <user:pass>` / `ZNIFF_PROXY_AUTH` |
| **中继上传** | `Authorization: Bearer <密钥>` | `--token <secret>` / `relay_token` |
| **中继查看** | HTTP Basic | `--view-auth <user:pass>` / `ZNIFF_RELAY_VIEW_AUTH` |

---

## 常用快捷键

| 快捷键 | 功能操作 |
| :--- | :--- |
| `Cmd/Ctrl + F` | 快速聚焦过滤输入框 |
| `Cmd/Ctrl + K` | 清空当前捕获会话的所有记录 |
| `Cmd/Ctrl + R` | 重放当前选中的流量 |
| `Cmd/Ctrl + Enter` | 发送构造的请求 / 放行当前断点请求 |
| `↑` / `↓` | 在过滤后的列表中切换选中行 |
| `Esc` | 收起编辑框或取消选中以查看列表全局统计 |

---

## 配置持久化与数据目录

用户配置与 CA 证书保存在对应系统的标准用户目录：
- **macOS**：`~/Library/Application Support/zniff`
- **Linux**：`$XDG_DATA_HOME/zniff` 或 `~/.local/share/zniff`
- **Windows**：`%APPDATA%\zniff`

### 核心文件列表
- `config.toml`：生产环境配置（默认抓包端口 `8877`）。
- `proxy-snapshot.toml`：系统代理崩溃自愈快照备份文件。
- `ca.crt` / `ca.key`：本地根 CA 证书与私钥。
- `config.dev.toml`：开发调试隔离配置（`make dev` 或 `ZNIFF_DEV=1`，默认端口 `8878`），与生产配置严格物理隔离。

---

## 本地开发与编译构建

在仓库根目录执行以下常见构建命令：

```bash
# 启动桌面开发模式并开启文件监控热重载（默认端口 8878）
make dev

# 构建高度优化的 Release 发行版二进制
make release

# 打包 macOS 原生应用程序包 (zniff.app)
make bundle

# 启动 Web 控制台（默认端口 8080）
make web

# 重新构建 WebAssembly 操作台资源包 (依赖 nsh 0.5)
make web-ui

# 运行自动化测试套件与静态代码分析
make test
make lint
```

---

## 开源许可证

本项目基于 **Apache License, Version 2.0** 协议开源。详细信息请参阅 [`LICENSE`](./LICENSE)。
