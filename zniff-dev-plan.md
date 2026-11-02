# zniff 开发流程：Rust + gpui 跨平台 HTTP(S) 抓包代理

> 编写日期：2026-11-02。库版本于当天在 crates.io API 核实。
> 定位：开源、原生、跨平台的抓包与调试代理；抓到的请求可以一键发到 cyberapi 重放。

---

## 1. 总体架构

### 1.1 Cargo workspace 布局
```
zniff/
├─ Cargo.toml                  # [workspace]，统一管理依赖版本
├─ crates/
│  ├─ zniff-core/              # 领域模型：Flow/Request/Response/Rule/事件类型，不依赖 IO
│  ├─ zniff-proxy/             # 代理引擎：监听、CONNECT、MITM、HTTP/1.1、HTTP/2、WebSocket、上游代理
│  ├─ zniff-ca/                # CA 生成、加载、叶子证书签发与缓存、导出（PEM/DER/p12）
│  ├─ zniff-rules/             # 规则引擎：过滤表达式、Map Local/Remote、重写、断点
│  ├─ zniff-store/             # 持久化：SQLite（元数据）＋ body 分块文件；会话导入导出
│  ├─ zniff-export/            # HAR 1.2、cURL、cyberapi 格式导出与联动
│  ├─ zniff-sys/               # 系统集成：系统代理开关/还原、安装/卸载根证书（按平台用 cfg 分模块）
│  ├─ zniff-capture/           # 进阶抓包后端（feature 开关）：windivert、ebpf(aya)、macOS NE 的 IPC 客户端
│  ├─ zniff-cli/               # 无界面模式：zniff run / ca / export / restore-proxy
│  └─ zniff-app/               # gpui + gpui-component 图形界面
├─ platform/
│  ├─ macos-netext/            # Swift 写的 Network Extension（透明代理），用 Xcode 工程单独构建
│  └─ linux-ebpf/              # aya 的 eBPF 内核态程序（bpf target），由 xtask 编译
├─ xtask/                      # 构建、打包、签名、编译 eBPF 等脚本（cargo xtask）
└─ tests/fixtures/             # 测试证书、HAR 样例
```

### 1.2 各模块职责与通信方式
- **zniff-proxy** 是一个独立的 tokio runtime，对外只暴露两样东西：
  - `ProxyHandle`：用于启动、停止，热更新规则和配置；
  - `broadcast::Receiver<FlowEvent>`：事件流，事件包括 RequestStarted、ResponseHeaders、BodyChunk、Completed、Error、WsMessage。
- **断点**：代理在命中断点时把 `oneshot::Sender<Decision>` 随事件一起发给 UI，UI 编辑完成后回传决定（放行、修改后放行、丢弃），并设置超时防止连接被永久挂起。
- **zniff-store** 订阅事件流，批量写入（每 100 条或每 50ms 提交一次事务）。body 超过 64KB 的部分写进分块文件，数据库里只存偏移量。
- **zniff-app 的 UI 线程不直接碰 IO**：
  - 后台任务把事件聚合成批次后，通过 gpui 的 `cx.spawn` 和 channel 推送到 Model，界面最多每 16ms 刷新一次；
  - 列表只保存轻量的 `FlowSummary`（id、方法、host、路径、状态码、大小、耗时）；
  - 打开详情时才从 store 按需加载 body。
- **zniff-cli** 复用 proxy、store 和 export，输出 JSON Lines 或 HAR，适合 CI 和远程服务器场景。
- **cyberapi 联动**：
  - 首选 URL Scheme / 深度链接，例如 `cyberapi://import?data=<base64 json>`；
  - 数据量大时退化为写临时文件，再用 `cyberapi://import?file=...` 传递；
  - 同时提供「复制为 cURL」作为通用兜底。
  - 交换格式需要和 cyberapi 一起定一份 JSON schema。好在两个项目都是 vicanso 自己的，可以直接在 cyberapi 里加导入入口。
- **平台抓包后端**：zniff-capture 把拿到的连接统一交给 zniff-proxy 处理（透明模式下目标地址由后端提供，而不是来自 CONNECT 请求）。
  - macOS 的 NE 扩展通过本地 socket（Unix domain socket，或 XPC 再桥接到 socket）把流量交给 Rust 主进程；
  - Windows 的 WinDivert 把流量改道到本地监听端口；
  - eBPF 后端只产出「明文事件」，走另一条只读事件流，不经过 MITM。

---

## 2. 关键选库（2026-11-02 核实）

| 用途 | 选用（版本） | 理由 | 备选 |
|---|---|---|---|
| 异步运行时 | tokio 1.54 | 生态事实标准，hyper/rustls 原生支持 | smol（gpui 内部用的就是 smol 风格执行器，但网络层还是 tokio 更全） |
| HTTP | hyper 1.12 + hyper-util 0.1.21 + http-body-util 0.1.5 | 1.x API 稳定；client 和 server 都能精细控制，支持 h1/h2 和 upgrade（WebSocket） | reqwest（只适合当上游 client，代理侧控制力不足） |
| HTTP/2 | h2 0.4.20（通过 hyper 间接使用） | hyper 的底层实现；需要时可直接用它处理 frame 级细节 | — |
| TLS | rustls 0.23.46 + tokio-rustls 0.26.6 | 纯 Rust、跨平台一致，支持 ALPN 和按 SNI 动态选证书（ResolvesServerCert） | native-tls / openssl（体积大、行为不一致，仅在需要老旧密码套件时考虑） |
| 上游证书校验 | rustls-platform-verifier 0.7.1 | 用系统信任库校验上游，行为接近浏览器 | webpki-roots |
| 证书签发 | rcgen 0.14.10 | 生成 CA 和按 SNI 签发叶子证书，社区活跃 | openssl crate |
| 证书解析 | x509-parser 0.18.1 | 展示上游证书链详情、校验导入的 CA | — |
| 参考实现 | hudsucker 0.25.0（2026-07 仍有更新） | 基于 hyper 1 + rustls 的 MITM 库，**先用它在 M0 快速跑通**，再按需 fork 或自研（它对断点、流式 body、h2 细节的控制有限） | 参考 mitmproxy 的 flow、addon、透明模式设计（Python，只借鉴设计） |
| WebSocket | tokio-tungstenite 0.30 | 握手升级后逐帧解析和转发 | 用 hyper upgrade + 自己解析帧 |
| DNS | hickory-resolver 0.26.3 | 支持自定义 DNS / DoH 和 hosts 覆盖（Map Remote 需要） | 系统 getaddrinfo（默认路径） |
| 存储 | rusqlite 0.41.1（bundled，WAL 模式） | 同步 API 简单，放到专用写线程里没有 async 负担；单文件会话便于导出 | sqlx 0.9（async 和编译期 SQL 校验，但对本项目偏重） |
| UI | gpui 0.2.2（crates.io 已发布，2026-10-30）+ gpui-component 0.8.1 | 虚拟化 Table/List、Dock 布局、代码编辑器、Markdown/JSON 高亮现成 | 必要时锁定 zed 仓库的 git rev；egui（备胎） |
| eBPF | aya 0.16.0 | 纯 Rust 的 eBPF，用户态和内核态同语言 | libbpf-rs |
| macOS 系统集成 | security-framework 3.7（钥匙串、信任设置）+ system-configuration 0.8（读系统代理） | 原生 API 绑定；写系统代理用 `networksetup` 命令或 SCPreferences | 直接调 `security` 命令行 |
| Windows 系统集成 | windows 0.62（WinINet InternetSetOption、Crypt32 证书 API、注册表） | 微软官方绑定 | winreg + 调用 certutil |
| Windows 透明抓包 | windivert 0.6.0（2025-09，更新较慢） | 绑定封装成熟度一般，必要时自己写 FFI；驱动需随包分发（WinDivert 为 LGPL/GPL 双许可，需注意） | WFP（需要自写驱动，成本极高） |
| CA 私钥保存 | keyring 4.2 | 跨平台访问钥匙串、Credential Manager、Secret Service | 文件 + 0600 权限（作为兜底） |
| 日志 | tracing 0.1.44 + tracing-subscriber + tracing-appender | 结构化日志，按天滚动 | log |
| 崩溃上报 | sentry 0.49.5（默认关闭，用户同意后开启） | 成熟，也可自建 | 只在本地写 crash dump |
| 打包 | cargo-packager 0.11.8（dmg/msi/AppImage/deb，内含更新器） | 一套配置覆盖三平台 | cargo-bundle、cargo-wix、手写脚本 |

> 版本策略：在 workspace 的 `[workspace.dependencies]` 里统一锁版本；gpui 依赖锁死具体版本或 git rev，每次升级单独开 PR 验证。

---

## 3. 平台策略

### 3.1 macOS
- **默认方式：系统代理 + 根证书**
  - 安装证书：用 security-framework 把 CA 加入登录钥匙串，并设置信任（`SecTrustSettingsSetTrustSettings`）。这一步会弹出系统授权框，这是预期行为。
  - 设置代理：对每个已启用的网络服务执行 `networksetup -setwebproxy/-setsecurewebproxy`，执行前先保存原配置。系统偏好会要求管理员权限，可以用 `AuthorizationExecuteWithPrivileges` 的替代方案，即 SMAppService 注册的 helper；MVP 阶段先直接调用 networksetup 并提示授权。
- **进阶方式：Network Extension 透明代理（NETransparentProxyProvider）**
  - 用 Swift 写一个 System Extension，主 App 负责激活它；扩展把每条 TCP flow 通过本地 Unix socket（或 XPC 再转 socket）交给 Rust 进程。协议可以很简单：长度前缀 + flow 元数据（进程 bundle id、pid、目标地址）+ 原始字节。
  - 前提条件：
    - 付费 Apple Developer 账号；
    - `com.apple.developer.networking.networkextension` entitlement（app-proxy-provider / transparent proxy）；
    - System Extension entitlement；
    - Developer ID 签名 + notarization；
    - 用户要在「系统设置 → 隐私与安全」里允许扩展。
  - 收益：能抓到不读系统代理的进程，并且能按进程过滤。

### 3.2 Windows
- **默认方式**
  - 设置代理：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings` 里的 ProxyEnable、ProxyServer、ProxyOverride，然后调用 `InternetSetOption(INTERNET_OPTION_SETTINGS_CHANGED / REFRESH)` 通知系统生效。
  - 安装证书：用 Crypt32 的 `CertAddEncodedCertificateToStore` 放入 CurrentUser\Root，会弹系统确认框；备选是 `certutil -user -addstore Root`。
- **进阶方式**
  - WinDivert（需要管理员权限和驱动）：按端口 80/443 和进程过滤后，把流量改道到本地监听端口，实现透明代理。
  - WFP 需要自写内核驱动和 EV 签名，**不列入计划**。

### 3.3 Linux
- **设置代理**
  - GNOME：`gsettings set org.gnome.system.proxy mode manual`，以及 http/https 的 host 和 port；
  - KDE：`kwriteconfig5/6 --file kioslaverc`，然后用 dbus 通知；
  - 其他环境：输出 `export http_proxy=... https_proxy=...` 片段，并提供「用代理启动命令」功能，即 `zniff run -- <cmd>` 自动注入环境变量。
- **安装证书**：把 CA 复制到 `/usr/local/share/ca-certificates/zniff.crt`，再执行 `update-ca-certificates`（Debian 系）；Fedora/Arch 用 `trust anchor` / `update-ca-trust`。需要 pkexec 提权。
- **进阶方式：aya eBPF 抓本机 TLS 明文**
  - 参考 eCapture：在 OpenSSL/BoringSSL 的 `SSL_read/SSL_write` 上挂 uprobe，读取明文缓冲区；Go 程序的 crypto/tls 要单独处理。
  - 局限：只能抓本机进程、只读、不能修改，静态链接或去掉符号的二进制可能失效；需要 root 或 CAP_BPF，内核 ≥ 5.8。
  - 在 UI 里单独标记为「被动明文」来源。

### 3.4 共性问题
- **退出或崩溃后还原代理**：
  - 修改代理前，把原始配置写入 `~/.zniff/proxy-backup.json`，并写一个 dirty 标记；
  - 正常退出、收到 SIGINT/SIGTERM、Windows 收到 `CTRL_CLOSE_EVENT` 时还原，panic hook 里也执行还原；
  - 下次启动时如果发现 dirty 标记，就先还原再提示用户；
  - 另外提供 `zniff restore-proxy` 命令，以及安装包里附带的「还原代理」快捷入口；
  - 可选 watchdog：一个小进程监控主进程，主进程消失就执行还原。
- **不读系统证书的程序**：
  - Firefox：提示开启 `security.enterprise_roots.enabled`，或用 certutil(nss) 导入；
  - Node：`NODE_EXTRA_CA_CERTS=<pem>`；
  - Java：`keytool -importcert` 导入 cacerts，或设置 `-Djavax.net.ssl.trustStore`；
  - Python requests：`REQUESTS_CA_BUNDLE`；
  - Go：通常读系统证书，但不读代理配置时要设置 `HTTPS_PROXY`；
  - curl：`--cacert`。
  - 「证书安装向导」页面给出一键复制的命令，`zniff run -- <cmd>` 自动注入这些环境变量。
- **证书固定（pinning）的 App**：
  - 检测到 TLS 握手失败（客户端发 alert 或连接立即断开）时，自动把该 host 加入「直通列表」，只记录 SNI 和流量大小，不解密，并在 UI 里提示；
  - 文档里说明：要绕过 pinning 只能在自己可控的应用里操作（调试版关闭 pinning，或 Android 的 network_security_config），**不提供绕过他人 App 的工具**。

---

## 4. 分阶段里程碑（单人业余开发，约每周 8–12 小时）

| 阶段 | 目标 | 交付功能 | 验收标准 | 粗估 |
|---|---|---|---|---|
| **M0 CLI 跑通 MITM** | 命令行版 HTTP/HTTPS 代理 | workspace 骨架；zniff-ca 生成和加载 CA、按 SNI 签发并缓存叶子证书；基于 hudsucker（或 hyper 自写）的 h1 代理；`zniff run --port 8888` 输出 JSON Lines；`zniff ca export` | `curl -x localhost:8888 --cacert ca.pem https://example.com` 返回正常并打出完整 flow；集成测试在三个平台的 CI 上通过 | 3–4 周 |
| **M1 gpui 最小可用版本** | 能日常用来看流量 | zniff-store（SQLite WAL + body 文件）；gpui 主窗口：虚拟列表、过滤搜索框、详情面板（Headers/Body/Raw，JSON 美化，图片预览）；开关系统代理和还原；证书安装向导 | 用浏览器正常上网 30 分钟不崩、不卡；退出后代理已还原；10 万条记录时滚动保持 60fps | 5–6 周 |
| **M2 规则、断点、导出、联动** | 从「看」进到「改」，并打通 cyberapi | 过滤表达式（host/path/status/method/content-type）；Map Local、Map Remote、Header 重写；请求/响应断点（编辑后放行）；导出 HAR 1.2 和 cURL，导入 HAR；**发送到 cyberapi**（深度链接 + 临时文件）；会话保存和打开 | 任选一条 flow 在 cyberapi 里打开，字段无损（方法、URL、headers、body）；导出的 HAR 能被 Chrome DevTools 导入 | 4–5 周 |
| **M3 HTTP/2 与 WebSocket** | 现代协议支持 | ALPN 协商 h2，客户端和上游两端都支持 h2；WebSocket 逐帧展示，支持文本、二进制和 ping；SSE 流式展示；gRPC 识别（先显示原始数据，Protobuf 解码放到后面） | h2 网站（如 Google 系）抓包正常，UI 标出协议；WS echo 测试双向帧完整 | 3–4 周 |
| **M4 透明代理与 eBPF（可选、分平台）** | 抓到不走系统代理的流量 | macOS NE 扩展 + socket 协议；Windows WinDivert 后端；Linux aya uprobe 抓 OpenSSL 明文；按进程过滤 | 三个平台各有一个「不读代理」的示例程序（例如设置了 NO_PROXY 的 curl）能被抓到；关闭功能后网络完全恢复 | 6–10 周（可以拆开逐个平台做） |
| **M5 打包、签名、自动更新与 1.0** | 可以分发给普通用户 | cargo-packager 产出 dmg（签名＋公证）、msi（代码签名）、AppImage/deb；自动更新（签名校验）；崩溃上报（需用户同意）；文档站和截图 | 三个平台在干净虚拟机上从安装、首次运行、装证书、抓包到卸载全流程通过；更新包验签失败会拒绝安装 | 3–4 周 |

合计：不含 M4 约 5–6 个月，含 M4 约 7–9 个月。建议 M2 完成后就发 0.x 公测版。

---

## 5. 工程实践

### 5.1 测试策略
- **单元测试**：zniff-core 的过滤表达式解析、HAR 和 cURL 序列化（快照测试用 insta）、规则匹配。
- **证书测试**：
  - 生成 CA 后签发叶子证书，用 rustls 的 WebPkiServerVerifier 以该 CA 为信任根验证通过（SAN 包含 IP 和通配域名、有效期、密钥用途）；
  - 加载损坏或过期的 CA 时必须报出清晰错误。
- **代理集成测试**（`tests/` 目录，全部在本机进行）：
  - 用 hyper 在随机端口起 HTTP/HTTPS/h2/WS 测试服务器，测试证书由 rcgen 现场生成；
  - 代理也在随机端口启动；
  - 用 reqwest 配置代理并信任测试 CA 发请求，断言响应内容和 FlowEvent 序列；
  - 覆盖分块传输、gzip/br、大 body（100MB 流式不爆内存）、上游超时和断连、断点放行与修改、Map Remote。
- **系统集成测试**：只在 CI 的一次性虚拟机上跑，设置代理后立即还原，并断言系统配置和备份一致；默认用 `#[ignore]` 跳过，通过 `cargo test -- --ignored` 手动触发。
- **UI**：手动测试清单为主，加上 gpui 的 TestAppContext 对 Model 层（过滤、选择、批量追加）做单元测试。
- **模糊测试**：cargo-fuzz 覆盖 HTTP 解析边界和过滤表达式解析器。

### 5.2 性能目标
- 10 万条 flow 时：列表滚动 ≥ 60fps，过滤响应 < 100ms（在内存里维护 FlowSummary 的列式索引），内存 < 500MB（body 不常驻内存）。
- 代理开销：本地 1000 rps 下 p99 额外延迟 < 5ms；签发叶子证书 < 5ms，并用 LRU 缓存。
- 冷启动 < 1s。
- 实现手段：gpui-component 虚拟 Table、事件批量合并刷新、body 懒加载、大 body 只显示前 N KB。
- 每个版本用 criterion 做基准测试，再加一个 `xtask bench-load` 压测脚本（oha/wrk 打流量）。

### 5.3 CI（GitHub Actions）
- 矩阵：`macos-14`（arm64）、`macos-13`（x64，或交叉编译）、`windows-latest`、`ubuntu-22.04`。
- 步骤：fmt → clippy -D warnings → test → build --release；用 Swatinem/rust-cache 缓存依赖。
- Linux 需要额外安装 gpui 依赖（libxkbcommon、wayland、vulkan 等）。
- eBPF 用单独的 job（nightly 工具链 + bpf-linker）。
- 打 tag 时触发 release 工作流：打包、签名、公证，上传到 GitHub Releases，并生成更新清单 latest.json（带签名）。
- 加上 cargo-deny 做许可证和安全公告检查，cargo-machete 清理未用依赖。

### 5.4 打包与分发
- **macOS**：`.app` → dmg；用 Developer ID 签名（hardened runtime），通过 `xcrun notarytool` 公证并 staple。带 NE 扩展时扩展必须内嵌在 `.app/Contents/Library/SystemExtensions`。
- **Windows**：msi（cargo-packager 的 WiX 后端）；代码签名证书可选 Azure Trusted Signing 降低成本。WinDivert 驱动随包分发，要单独签名并检查许可证。
- **Linux**：AppImage + deb（必要时加 rpm）。提权操作（装证书）通过 pkexec 调用小脚本完成。
- **自动更新**：用 cargo-packager 的更新器或自写——检查 latest.json，校验 minisign/ed25519 签名后再替换。Linux 的 deb 用户交给包管理器，AppImage 用 zsync 或自身更新器。

### 5.5 日志与崩溃
- 日志写到 `~/.zniff/logs/`，按天滚动、保留 7 天；默认不记录 body，避免泄露敏感数据。
- 崩溃时，panic hook 先还原代理，再写本地崩溃报告；sentry 只在用户明确同意后启用，并在上报前脱敏 URL 查询参数和 headers。

### 5.6 安全
- 每台机器首次运行时独立生成 CA（ECDSA P-256，有效期 1–2 年，Name Constraints 不可行，所以把 CN 写明为 "zniff Local CA - <hostname>"），**绝不内置统一 CA**。
- CA 私钥用 keyring 存进系统钥匙串（macOS Keychain、Windows Credential Manager/DPAPI、Linux Secret Service）；没有 Secret Service 时退化为 0600 权限的文件并提示风险。
- 提供「重新生成 CA」和「卸载 CA」功能，卸载程序时提示移除证书。
- 代理默认只监听 127.0.0.1；开放给局域网（手机抓包）需要用户手动开启，并支持可选的访问口令。
- 会话文件可能含 token，导出 HAR 时提供「脱敏 Authorization/Cookie」选项。

### 5.7 开源许可证
- 建议 **Apache-2.0**（和 gpui-component 一致，带专利授权，商业友好）。也可以选 MIT/Apache 双许可，或者为防止被闭源分发而选 GPL-3.0。
- 注意依赖许可证：gpui 是 Apache-2.0；WinDivert 是 LGPL-3.0/GPL-2.0，动态加载并随包附上许可证文本即可。用 cargo-deny 守住许可证。

---

## 6. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| gpui API 不稳定、文档少 | 升级就编译失败，有被迫重构的成本 | 锁定版本；UI 层和业务 Model 分离；参考 gpui-component 的 story 和 Zed 源码；升级单独开分支 |
| 代理异常退出导致用户断网 | 口碑问题最严重 | 先备份、写 dirty 标记、启动时自检还原、提供 restore-proxy 命令、watchdog，并纳入 M1 验收 |
| HTTPS 兼容性（h2、ALPN、客户端证书、老 TLS、pinning） | 部分网站和 App 抓不到或直接打不开 | 失败自动转为直通，UI 给出原因；上游支持客户端证书配置；维护一份兼容性测试站点清单 |
| macOS NE 和公证门槛（账号、entitlement、审核） | M4 可能卡住 | 默认路径不依赖 NE；NE 作为可选项，较晚做；提早申请 entitlement |
| Windows 签名和 WinDivert 驱动 | SmartScreen 警告，杀毒软件误报 | 买或使用云签名；WinDivert 作为可选组件；提交给杀毒厂商白名单 |
| eBPF 适配面广（内核版本、TLS 库、静态链接） | 投入产出比低 | 只支持系统 OpenSSL 的动态链接场景，标记为实验功能 |
| 大流量下内存和 UI 卡顿 | 体验崩溃 | body 落盘、列表只存摘要、批量刷新；设置会话上限并支持自动滚动清理 |
| 单人业余开发，范围膨胀 | 迟迟发布不了 | 严格按里程碑推进，M2 后发公测；M4 拆分，按用户反馈再决定做不做 |
| 安全责任（CA 私钥泄露等于中间人风险） | 用户信任 | 每机独立 CA、私钥进钥匙串、一键卸载，并在文档中写清楚威胁模型 |

---

## 7. 第一周任务（可直接拆成 issue 的粒度，仅列出，不提交）
1. 初始化 cargo workspace：建好 zniff-core、zniff-ca、zniff-proxy、zniff-cli 四个 crate，设置 `[workspace.dependencies]`、rust-toolchain.toml、rustfmt 和 clippy 配置。
2. 添加 GitHub Actions：三个平台矩阵跑 fmt、clippy、test，配置 rust-cache。
3. 添加 LICENSE（Apache-2.0）、README 骨架、cargo-deny 配置。
4. zniff-ca：`Ca::generate()` 生成 ECDSA P-256 自签 CA（rcgen），以 PEM 读写到 `~/.zniff/ca/`；补单元测试。
5. zniff-ca：`Ca::issue(sni)` 签发叶子证书（SAN 支持域名和 IP，有效期 ≤ 397 天），加 LRU 缓存；用 rustls 的 verifier 验证通过的测试。
6. zniff-proxy：基于 hudsucker 搭最小 MITM 代理（监听 127.0.0.1:8888，接入 zniff-ca 的签发逻辑），打印请求行和状态码。
7. zniff-core：定义 `Flow`、`FlowEvent`、`FlowSummary` 结构（serde），代理通过 broadcast channel 发出事件。
8. zniff-cli：`zniff run --port --jsonl` 和 `zniff ca export --format pem|der`（基于 clap）。
9. 集成测试：用 rcgen 和 hyper 起本地 HTTPS 测试服务器，用 reqwest 经代理请求，断言响应内容和事件序列。
10. 技术预研：写一个 gpui-component 的虚拟 Table demo，塞入 10 万行假数据，测滚动帧率并记录结论。
11. 文档：写 `docs/cyberapi-import.md`，起草 zniff→cyberapi 的 JSON 交换格式和深度链接方案（在 cyberapi 侧对应添加入口）。
12. 技术预研：macOS 和 Windows 上设置/还原系统代理的最小脚本（networksetup / 注册表），验证备份和还原流程。
