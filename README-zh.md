# zniff

[English](./README.md) | 简体中文

**zniff** 是一款使用 Rust 编写的轻量级原生桌面 HTTP(S) 抓包与调试代理工具。代理核心（`hyper`、`rustls`、`tokio`）与原生图形界面（`nsh`）运行在同一进程内，无外部数据库依赖，启动迅速、资源占用可控。

---

## 核心特性

### 1. 协议跟随的 HTTP(S) 与 WebSocket 代理
- **协议跟随（HTTP/1.1 & HTTP/2）**：解析客户端 TLS `ClientHello` 中的 ALPN 列表并直接与源站握手，客户端与源站两侧严格使用源站选定的协议，不强制降级为 HTTP/1.1，不合并 `Cookie`，也不篡改业务请求头的大小写。
- **旁路解压与大体积 Body 环形落盘**：流量转发不等待界面渲染或解压记录。请求与响应正文在旁路按 `Content-Encoding`（`gzip`、`deflate`、`br`、`zstd`）反序解压并记录（单条上限 `1 MB`）。未星标请求完成后若正文超过 `16 KB`，会自动顺序追加写入系统临时目录 `zniff-spill/` 下的环形分片文件（单分片 `64 MB`，最多 8 个分片共 `512 MB`，Unix 下打开即 `unlink` 匿名化以保证进程强退后由内核自动回收），内存会话上限控制在 `256 MB`。
- **WebSocket 与 SSE 消息解析**：从旁路实时解析 WebSocket 帧（文本、二进制、Ping/Pong、Close），转发握手时自动剥离 `Sec-WebSocket-Extensions` 以避免压缩协商导致无法解析；对 `text/event-stream` 响应自动按 SSE 事件拆分展示。
- **直通名单与隧道回退**：支持按域名通配符（`*.example.com`）或白名单（`only:`）配置 TLS 直通（Bypass）；`ServerHello` 之前的握手失败自动回退为透明 TCP 隧道，不中断客户端正常访问。
- **上游代理串联**：支持配置 `upstream_proxy` 将流量转发至二级 HTTP/CONNECT 代理。

### 2. 高效列表与多维过滤语法
- **可定制表格列**：搜索框右侧列按钮与表头右键菜单按表格列顺序一对一控制：`序号`、`方法`、`路径`（可切换为完整 `URL`）、`状态`、`间隔`、`耗时`、`大小`、`协议`、`主机`、`来源 IP`。支持直接拖拽表头或在列设置面板中点击上移/下移按钮调整列顺序（支持一键恢复默认列序），可见的 `序号`、`方法`、`路径` 固定在左侧区域，其余列在右侧滚动区域横向滚动，所有列宽与列序持久化记忆，路径与主机支持开头/中间/结尾省略切换。
- **组合过滤表达式**：空格分隔多个 `AND` 条件，单个条件内支持用 `|` 或 `,` 表示 `OR`：
  - `host:api.example.com|*.cdn.com`（或 `domain:`）：多域名与泛域名匹配
  - `path:/v1|/auth`：多路径前缀匹配
  - `ip:127.0.0.1|192.168.1.10`（或 `client:`）：客户端来源 IP 匹配
  - 方法精确匹配：`GET`、`POST`、`PUT`、`DELETE`、`CONNECT` 等
  - 状态码匹配：精确状态码（`200`、`404`）、状态段（`1xx`–`5xx`）或失败请求（`失败` / `fail` / `error` / `err`）
  - 类型匹配：`type:json`（或 `ct:html`、`ct:image` 等）
  - 内容搜索：`body:keyword`（搜索请求与响应正文）、`header:token`（搜索请求/响应头名与头值）
  - 快捷标记：`pin:`（仅看星标）、`slow:`（耗时 ≥ 1 秒的慢请求）
  - 排除条件：`mute:ads.example.com` 或 `-keyword`（排除匹配的主机或路径）

### 3. 规则改写、断点调试与配置预设
- **终点规则**（按文本顺序第一条命中生效）：
  - `map-local <匹配模式> <本地文件路径> [status=200]` — 直接用本地文件响应，不连接源站。
  - `map-remote <匹配模式> <https://目标主机/路径>` — 将请求转发至新的目标地址。
  - `break <匹配模式>` — 在请求发往源站前暂停，可实时修改方法、URL、请求头与请求正文后放行或丢弃。
  - `break-res <匹配模式>` — 在响应返回客户端前暂停，可实时修改状态码、响应头与响应正文后放行。
- **叠加修饰规则**（所有命中项叠加生效）：
  - `replace <匹配模式> req|res <原文> -> <替换为>` — 替换请求路径/查询串/正文或 UTF-8 响应正文。
  - `delay <匹配模式> <ms|s>` — 注入最高 30 秒的请求延迟。
  - `header <匹配模式> req|res set|del <HeaderName>[=Value]` — 增删改请求头或响应头。
  - `cors <匹配模式>` — 自动补齐跨域响应头，并对无终点规则的 `OPTIONS` 预检请求直接返回 `204`。
- **配置预设（`SavedProfile`）**：顶栏「过滤」双栏面板支持将过滤条件、直通名单、规则文本、上游代理与开关状态保存为命名配置（最多 16 条），支持导出为 `.toml` 文件或通过文件选择框/拖拽 `.toml` 导入。

### 4. 详情检视、基准对比与编辑重发
- **多视图正文展示**：
  - JSON、HTML、XML 及多行文本在带行号与代码折叠的只读编辑器中高亮展示。
  - 对 HTML 与 Markdown 正文提供「预览 / 美化 / 原文」三态切换，通过 `text_view` 直接渲染富文本结构。
  - 图片响应（`png`、`jpeg`、`webp`、`gif`）在美化模式下直接按原图比例预览并标注尺寸与字节数。
  - 二进制正文提供带行号的 16 字节 / 32 字节自适应宽度十六进制视图。
  - 请求头、查询参数与表单字段中的 **JWT**、10/13 位**时间戳**与可打印 **Base64** 自动在原值下方展示解码提示。
- **行级基准对比（Diff）**：将任意一条流量设为「基准」（红竖线标记），选中另一条流量即可在「对比」卡片中上下对照两者的 URL、请求头、请求正文、响应头与响应正文差异。
- **重放与编辑重发**：支持一键重放（`Cmd/Ctrl+R`）或打开「编辑重发」修改方法、URL、头与正文；在 URL 输入框中直接粘贴 `curl ...` 命令可自动解析并回填各字段。
- **多格式复制**：支持复制 URL、`cURL`、`fetch`、Python `requests`、响应 JSON 的浅层 TypeScript `interface`、请求原文与响应原文，以及一键复制终端代理环境变量（`http_proxy`、`SSL_CERT_FILE`、`NODE_EXTRA_CA_CERTS` 等）。

### 5. 手机扫码抓包、HAR 与实时网页分享
- **系统代理与根证书管理**：自动生成 `zniff CA` 根证书，支持在「更多」菜单中一键安装或移除证书（支持 macOS 登录钥匙串、Windows 当前用户 Root 存储、Linux `~/.pki/nssdb`）。系统代理修改前会将原配置独立备份至 `proxy-snapshot.toml`，即使进程异常退出，下次启动时也会自动恢复系统代理。
- **局域网监听与手机扫码装证**：打开底栏「局域网」开关绑定 `0.0.0.0` 后，点击「扫码」即可生成指向 `http://<局域网IP>:<端口>/zniff` 的二维码，手机扫码即可下载安装根证书并查看 iOS / Android 配置引导。
- **HAR 1.2 与 HTML 导出**：支持导入 `.har` 文件（菜单选择或直接拖拽到窗口），以及将当前筛选流量导出为 `.har` 或单文件离线 `.html` 快照。
- **实时网页分享**：支持生成 15 分钟、1 小时或 8 小时有效期的只读分享链接 `http://<地址>:<端口>/s/<uuid>`，网页端每 2 秒增量拉取新流量，并在点击条目时按需加载落盘的大体积正文。

---

## 快捷键

| 快捷键 | 功能 |
| :--- | :--- |
| `↑` / `↓` | 在当前筛选后的列表中切换上一条 / 下一条流量 |
| `Cmd/Ctrl + F` | 聚焦左侧过滤搜索框 |
| `Cmd/Ctrl + K` | 清空当前整个抓包会话 |
| `Cmd/Ctrl + R` | 重放当前选中的流量 |

---

## 配置文件与数据目录

用户配置保存在 `config.toml`，系统代理现场快照保存在 `proxy-snapshot.toml`：

- **macOS**：`~/Library/Application Support/zniff`
- **Windows**：`%APPDATA%\zniff`
- **Linux**：`$XDG_DATA_HOME/zniff` 或 `~/.local/share/zniff`

> **开发与正式环境隔离**：在 Debug 构建或设置环境变量 `ZNIFF_DEV=1`（如执行 `make dev`）时，程序会读写独立的 `config.dev.toml`（默认端口 `8878`）与 `proxy-snapshot.dev.toml`，与正式版 `config.toml`（默认端口 `8877`）互不干扰，同时共用 `ca.crt` 与 `ca.key` 根证书。

---

## 开发与构建命令

在仓库根目录下执行（`app/` 为唯一的 crate 与 workspace）：

```bash
# 开发模式运行（bacon 热重载，默认端口 8878）
make dev

# 编译 Debug 二进制
make build

# 代码格式化、Clippy 静态检查（拒绝任何警告）与单元测试
make fmt
make lint
make test

# 编译 Release 二进制
make release

# 打包 macOS 应用程序束（zniff.app）
make bundle
```

关于各平台能力边界与已接受的设计取舍，请参考 [docs/known-issues.md](./docs/known-issues.md)。

---

## 开源许可

本项目基于 [Apache License 2.0](./LICENSE) 许可开源。
