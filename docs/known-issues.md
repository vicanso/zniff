# zniff 已知限制与行为边界

> 返回 [文档索引](./README.md) · 产品取舍决策见 [产品与架构决策记录 (ADR)](./open-questions.md) · 规则语法见 [规则与配置指南](./rules-zh.md)

本文档维护 **zniff（当前基线 `v0.4.0` / `nsh 0.6.1`）** 明确接受的架构边界、平台限制与关键行为约束。核心阶段与 v0.4 规划能力已全部落地，见文末[已落地里程碑速查](#6-已落地里程碑速查)。

---

## 1. 明确不做的边界（Non-Goals）

以下能力超出 `zniff` 的轻量单进程显式 HTTP 代理定位，除非明确提出新需求，否则不纳入实现：

- **HTTP/3 与 QUIC**：UDP 流量不经过显式 HTTP 代理。
- **透明代理（TUN/TAP）与网卡级随机丢包模拟**：保持显式代理模型；应用层带宽限流已由 `throttle` 规则覆盖，不做内核层丢包/乱序模拟。
- **Firefox 证书库**：Firefox 默认不代理 `localhost` 且使用独立证书库；验收浏览器为 Chrome、Safari、Edge（及免证书隔离 Chromium）。
- **SQLite 持久化会话与 cyberapi**：会话为内存 + 临时环形分片，重启清空。
- **重型脚本运行时与抓包热路径内嵌脚本（Node / Python / `nsh-engine/js` / `swc` / `oxc`）**：代理转发与 10k/100k 列表热路径不跑脚本（动态改写继续使用纯 Rust `rule_vars.rs` + 本机回环 `hook`）；仅在非 `wasm32` 服务端/桌面端内置轻量沙箱 `quickjs-jit` 懒加载后台池（布局限 `8 MiB / 25 ms`，按需包转换限 `16 MiB / 80 ms`，连续超时 3 次自动熔断，闲置 60s 或最小化到托盘回收），保持桌面体积极轻且 `wasm32` 零 JS VM 膨胀。
- **正则（Regex）过滤与正则替换**：多域名 OR、多 Path 前缀 OR 与 `*` 通配已覆盖绝大多数场景，避免正则转义负担与全量流量匹配开销。
- **JSON 树级 Diff 与非 JSON 类型推导**：详情正文支持交互式 JSON 树形视图，但**基准对比（Diff）**固定为 URL / 请求头 / 请求体 / 响应头 / 响应体的行级对比（每边上限 200 行）；非 JSON 响应不推导 TypeScript / Rust 类型。
- **超过 `1 MB` 的正文改写与断点缓冲**：响应断点、`replace` / `json` 改写与 `hook` 载荷统一受 `BODY_CAP = 1 MB` 约束，超大响应保持流式转发。
- **完整 cURL 参数与 `jsAppend://` 专用语法**：粘贴 cURL 支持行尾 `\`、`$'...'`、`--json`、`-G`/`--get`、`-I`/`--head` 与多段 `-d`，不解析读取本地磁盘文件的 `-F`/`--form`（未识别旗标忽略）；网页脚本注入请直接使用 `replace ... res </body> -> ...`。
- **完整表格行规则编辑器与拖拽排序**：规则区保持纯文本一行一条（配合行号校验与单行辅助条：单行 `#` 启停、回填快速添加表单、删除），不拆成重型表格行编辑器。

---

## 2. 资源上限与存储边界

| 维度 | 当前上限 / 行为 | 说明 |
| :--- | :--- | :--- |
| **单条正文记录上限 (`BODY_CAP`)** | `1 MB`（解压后） | 超出部分安全截断（防御压缩炸弹），保留截断说明；仅未截断的非空图片触发预览。 |
| **大体积正文落盘 (`INLINE_BODY_CAP`)** | `> 16 KB`（已完成且未星标） | 顺序追加至系统临时目录 `zniff-spill/` 环形分片（单分片 `64 MB`，最多 8 分片共 `512 MB`，闲置 30 分钟回收；Unix 打开后立即 `unlink` 确保强杀由内核回收）。 |
| **内存会话上限 (`SESSION_CAP`)** | `256 MB` | 超出时淘汰最旧的未星标流量；线字节与 WebSocket 消息文本不计入该上限。 |
| **星标保护上限 (`PIN_CAP`)** | 最多 `32` 条 | 加星标时若正文已落盘会提回内存并豁免淘汰，因此星标集合可使总内存略超 `256 MB`。 |
| **WebSocket / SSE 消息上限** | `200` 条或 `256 KB` 文本 | 单条流量内超限后停止追加，不切断连接转发。 |
| **正文展示窗口 (`PREVIEW_CAP`)** | UTF-8 `256 KB`/页；二进制 `4 KB` 十六进制 | UTF-8 支持标题栏「往前 / 往后」按字符边界翻阅至 `1 MB`；复制与另存均基于完整记录原文。 |
| **Web 操作台列表 (`MAX_LIST`)** | 每次返回最近 `2000` 条 | `/api/flows` 支持 `?filter=` 服务端全字段过滤；导出支持 `/api/export.har`、`/api/export.html`、`/api/export.openapi.json`、`/api/export.openapi.yaml` 及 `/api/flows/{id}/body`。 |
| **实时分享 (`flows.json`)** | 保留最近 `400` 条 | 超过 `16 KB` 的落盘正文在点选详情或基准对比时经 `/s/{uuid}/flows/{id}.json` 按需加载。 |
| **配置预设与历史** | 预设最多 `16` 条；历史 `10` 条 | 分别存储在 `config.toml` 的 `saved_profiles` 与 `profile_history`；主过滤与布局等高频变更防抖半秒落盘。 |
| **规则分组与构造历史** | 规则组最多 `32` 组；构造历史最多 `20` 条 | 分别存储在 `config.toml` 的 `rule_groups` 与 `compose_history`；工作区 `rules` 优先级高于启用的规则组。 |
| **开发模式配置隔离** | `config.dev.toml` / 端口 `8878` | `debug_assertions`（非单测）或 `ZNIFF_DEV=1` 自动隔离配置与代理快照，共用 `ca.crt`/`ca.key`。 |

---

## 3. 平台与环境限制

### 3.1 桌面系统代理与证书

- **系统代理默认关闭**：仅当「更多 → 系统代理」打开时，点击「开始」才改写系统代理；若上次关闭窗口时处于抓包状态，下次启动会自动恢复抓包。
- **外部根证书自动探测与复用（`external_ca.rs`）**：桌面端可自动探测本机已安装且受信任的 **Whistle**、**mitmproxy**、**mkcert**、**Proxyman** 根证书及私钥并在界面一键切换复用（记入 `config.toml` 的 `external_ca`）；未检测到、未受信任或选回内置证书时均回退至本地 `zniff CA`。
- **macOS**：
  - `security add-trusted-cert` 可能需要用户弹窗确认；信任状态结合 `security find-certificate` 与 `security verify-cert -c <ca.crt> -L` 区分「未安装」「已安装但未信任」「已信任」（移除时一并清理旧名 `Snoop CA`）。
  - 带密码认证的系统代理无法经 `networksetup` 读回密码，停止抓包后密码项可能恢复不完整。
  - iOS 模拟器支持通过 `xcrun simctl` 向全部 `Booted` 模拟器写入信任根证书并设置代理（退出或停抓包时自动清理）；模拟器内 App 需重启读取新代理，真机仍需手动安装证书并配置 Wi-Fi 代理。
- **Windows**：系统代理仅修改当前用户的 **WinINET**，不改 **WinHTTP**；证书检测/移除使用 `certutil -user -verifystore / -delstore Root`。
- **Linux**：
  - 仅 GNOME 支持自动改写系统代理；非 GNOME 环境开启「系统代理」时仅启动监听并在状态栏提示手动指向 `127.0.0.1:<端口>`。
  - 证书管理默认操作 Chromium 系 NSS 库（`~/.pki/nssdb`），不直接改写系统 `/etc/ssl/certs`；菜单提供按发行版生成的系统证书命令一键复制。
- **窗口状态恢复**：窗口宽高（默认 `1200×760`）、屏幕坐标（`window_x` / `window_y`）、显示器标识（`window_display_id`）及窗口模式（`window_mode`：normal / maximized / fullscreen）均通过 `nsh_core::element::desktop::window_state()` 防抖半秒持久化，并在启动时由 `MainWindowOptions` 恢复；若上次记录的显示器已拔出，自动居中回落主屏。

### 3.2 Web 操作台（`zniff --web` / `wasm32`）

- Web 端与桌面端共用同一份 `App`（编译为 Wasm，由 `make web` 从 `app/.nsh/web` 提供；缺少产物目录时回落内置最小应急页）。
- 浏览器不直接抓包、不修改系统代理，也不暴露拉起本地外部进程的入口（如免证书浏览器/终端/`adb`/批量本地 Mock 落盘）；抓包端口仅在点击「开始」后由 `zniff --web` 服务端绑定。
- `wasm32` 目标不可直接调用 `std::env::temp_dir()`、`std::process::id()`、`SystemTime::now()` 或 `Instant::now()`；墙钟统一经 `clock::now()` 封装，大正文环形落盘仅在服务端/桌面进程执行。

### 3.3 自动更新（`updater.rs`）

- 仅从 **GitHub Releases** 检测与下载（已移除 Gitee 回退），全平台强制校验 Release 附带的 `SHA256SUMS`——缺失清单、缺少对应安装包条目或哈希不匹配一律拒绝安装并清理临时下载文件。

---

## 4. 代理内核、TLS 与规则边界

### 4.1 连接池、HTTP/2 与重放

- **上游代理与认证**：未配 `upstream_proxy` 时直连源站（不走系统代理）；支持 HTTP/CONNECT 与 `socks5://` / `socks5h://`（RFC 1928 + RFC 1929）。配置 `proxy_auth`（或 `--proxy-auth` / `ZNIFF_PROXY_AUTH`）后，抓包端口要求 HTTP Basic `Proxy-Authorization`（失败回 `407` 且不写入会话，转发前剥离该头；`/zniff`、`/zniff-ca.crt`、`/proxy.pac`、`/zniff.pac`、`/s/{uuid}`、`/r/{uuid}` 与回环 `/mcp` 豁免）。
- **代理域名准入控制（`allow_hosts.rs`）**：非严格模式下留空放行全部；启用 `--strict-allow-hosts` / `ZNIFF_STRICT_ALLOW_HOSTS=1`（Docker 镜像默认开启）且无正向允许项时拒绝所有代理域名。优先级为 `--allow-hosts` > 非空 `ZNIFF_ALLOW_HOSTS` > `config.toml` 的 `allow_hosts`，由 CLI 或非空环境变量传入时界面与 `/api/hub` 仅只读展示并拒绝写操作（`403`）。全链路覆盖 `CONNECT` 握手前（先于 `200` 与直通名单）、SNI、HTTP/HTTPS/H2、`map-remote` 目标主机与重放（同步过滤 `/proxy.pac`）；被拒绝请求直接回 `403 Forbidden`（不拨号、不写入会话），其有界计数与域名摘要（最多追踪 64 个域名）实时展示在底栏「安全」按钮角标、安全弹窗及设置安全页中（未锁定时支持一键 `+ 允许` 与清零），避免污染会话列表。本地路由 `/zniff`、`/zniff-ca.crt`、`/proxy.pac`、`/zniff.pac`、`/mcp`、`/s/`、`/relay` 豁免。
- **明文 HTTP 连接池**：客户端省略 `Connection` 时补 `Connection: keep-alive`；body 读完、双方未要求关闭且状态非 `101` 时回池（每键最多 4 条，超过 32 键清空）。HTTP/1 TLS 不进此池。
- **HTTP/2 连接复用**：同一客户端连接复用源站连接，另外缓存最多 16 条发送端（空闲超过 45 秒或驱动结束时淘汰）；仅在请求尚未写出时自动换连重试。HTTP/2 传输层不解析底层二进制帧、HPACK 表或 trailer。
- **重放独立建连**：重放/批量重放不走规则和断点、不进入连接池；HTTP/1.1 强制 `Connection: close`，源流为 `HTTP/2` 或新建 HTTPS 请求时宣告 `["h2", "http/1.1"]` ALPN 并按协商结果发送。

### 4.2 TLS 探测、错误推断与 WebSocket

- **TLS 失败退回隧道**：源站 TLS 探测失败会关闭探测连接、另开 TCP 重放 `ClientHello` 转为透明隧道，默认不在列表刷屏（打开「隧道失败」`show_tunnel_failures` 后记录）。`ServerHello` 之后的解析失败只记 `ERR`，无法退回隧道。
- **证书钉扎与错误原因猜测**：`ServerHello` 后客户端断开会按告警类型与常见钉扎域名给出启发式猜测（非专用钉扎检测）；转发失败会在步骤名后附按错误类型推断的短句，已落盘旧行保持写入时的文案。
- **WebSocket**：转发握手时强制移除 `Sec-WebSocket-Extensions` 避免协商 `permessage-deflate`；遇非法帧或 `RSV1 != 0` 仅停止解析后续帧，不切断数据对拷。

### 4.3 规则与断点边界

- **17 类规则生效顺序**：
  - 终点规则（`map-local`、`map-remote`、`break`、`break-res`、`block`）自上而下首条命中生效；
  - 修饰/扩展规则（`replace`、`json`、`header`、`query`、`delay`、`throttle`、`cors`、`no-cache`、`hook`）顺序叠加；
  - 域名级与只读解码规则（`cert`/`client-cert`、`resolve`、`proto`）按主机/路径生效。
  - WebSocket 升级后的数据帧与 `CONNECT` 隧道不套用 HTTP 改写规则；明文在拨号前应用，HTTPS 在源站连接打开后应用。
- **Chunked 改写与冲突**：仅当流量明确命中 `replace` 或 `json` 时，才对无 `Content-Length` 的 `chunked` 报文按需有界缓冲 `<= 1 MB` 解压改写并重算 `Content-Length`；未命中改写规则或超限时保持流式直通（超限跳过写入备注）。同时命中响应断点时，该轮不提前执行 `replace`，由断点查看源站原文。
- **动态变量与本地钩子**：
  - `{{...}}` 模板变量作用于 `header`、`query`、`replace`、`json`（不作用于 `map-local`）；未识别的 `{{...}}` 原样保留。
  - `hook` 严格限定本机回环 HTTP（`127.0.0.1` / `localhost` / `[::1]`）直连调用，单次超时 `50ms..=30s`（默认 `2000ms`），失败时 fail-open 并写备注；`mode=mutate` 协议使用 `body_base64` / `body_utf8`，`mode=preview` 使用 `body_b64` / `body`。
- **断点队列**：请求与响应断点共用 `BREAK_QUEUE_CAP = 8` 有界 FIFO 队列（满载自动放行队首最旧项）；队首默认 60 秒无操作自动放行（编辑中延长至 5 分钟）。响应断点仅缓冲 `<= 1 MB`；已知 `Content-Length > 1 MB` 时跳过暂停并写备注，未知长度读超 `1 MB` 则该条记失败。点击「丢弃」直接关闭连接/重置流，记 `ERR · 断点丢弃`。

---

## 5. 界面、交互与检视边界

- **布局与尺寸约束**：
  - 支持「左右分栏（`Split`，默认，左侧含可拖宽 `176–340 px`、可通过底栏按钮折叠的域名/应用侧栏）」与「上下分栏（`TopBottom`，宽屏详情将请求卡片排在左侧、响应卡片排在右侧）」。
  - 左栏列表默认宽 `660`（范围 `280–1100`）；`Split` 侧栏展开时上限受 `window_w - (sidebar_px + 406)` 约束以保留详情区空间，否则受 `window_w - 340` 约束。
  - 过滤弹出层宽度按 `window_w - list_px * 0.7 - 16` 在 `680–1360` 自适应，规则编辑器高度按 `84–460` 自适应。
  - **macOS `nsh 0.7.0` 隐藏标题栏输入焦点约束**：在 macOS 原生窗口（`TitleBarStyle::Hidden`、`42 pt` 顶部栏）中，当任意单行 `NSTextField`（如侧栏/列表过滤或预设名称输入框）处于焦点编辑态时，AppKit 会拦截落在顶部栏区域内的指针点击（工作区与底栏按钮不受影响），按 `Tab` 或 `Esc` 退出输入框编辑后即可恢复顶栏点击；点击预设弹窗「保存」与快速规则切类已主动调用 `dismiss_keyboard()` 释放焦点。
- **列表列与过滤**：
  - 共 10 列：默认开启 `序号、方法、路径、状态、间隔、耗时、大小`（7 列），默认关闭 `协议、主机、来源 IP`（3 列）；`路径` 默认宽 `300`，`主机` 默认宽 `88`。
  - 路径显示模式（`path_display`）支持 `auto`（默认：未开主机列且无生效正向 `host:`/`domain:` 过滤时显示域名+路径，否则仅显示路径）、`path_only`、`host_and_path`、`full_url`（兼容旧 `show_full_url`）；省略模式（`ellipsize`）支持 `head` / `middle`（默认） / `tail`。
  - `host:` / `domain:` 过滤为**精确（忽略大小写）或通配符（`*.` / `*`）匹配**，非子串匹配；子串查找请用裸词。
  - 方向键 `↑` / `↓` 切换选中行时自动将选中行滚入列表可见视口；`make dev` 热重载可能拦截 `Cmd+R`；全局命令面板快捷键为 `Cmd/Ctrl+Shift+P`（基于 `nsh_base::command`）。
- **字体与编码**：
  - 代码与表格等宽列默认使用随包压缩的 `JetBrains Mono NL`（可在「更多」切回系统等宽）；该字体无中文字形，中文回落系统字体。HTML 快照与 Web 端不内嵌字体。
- **详情预览与图片**：
  - 支持 `美化 | 树形 | 原文`（JSON 含 GraphQL / MsgPack / `.proto` 解码时开启「树形」标签；可在「更多」或命令面板切换 `default_json_tree`），HTML 与 Markdown 各自带独立「预览」开关。
  - 图片预览支持 `png` / `jpeg` / `webp` / `gif` / `ico`（原图尺寸展示，任一边超过 `200 px` 等比缩小），**不预览 SVG**。
  - 卡片折叠状态仅在当前窗口生命周期内保留，不持久化至 `config.toml`。
- **扫码与局域网监听**：
  - 监听默认 `127.0.0.1:8877`，端口被占用直接报错不静默换端口。
  - 仅本机监听模式下底栏「扫码」入口保持可见，点击后提示并支持一键切换至局域网监听（`0.0.0.0`）后展示安装页二维码。

---

## 6. 已落地里程碑速查

以下规划阶段的核心能力均已实现并具备单元/代理往返测试覆盖（源码位于 [`app/src/`](../app/src/)）：

| 阶段 | 核心交付 | 关键模块 |
| :--- | :--- | :--- |
| **核心基线** | 单进程 MITM（HTTP/1.1 + HTTP/2 ALPN 跟随 + WebSocket/SSE）、`zniff-spill` 环形分片、HAR/HTML 导出与结构化脱敏（含 `multipart` 与 `redact_strict`）、`/s/{uuid}` 实时分享与 `--relay`、同构 Wasm `--web` | `proxy/`、`session.rs`、`ca.rs`、`redact.rs`、`share.rs`、`har.rs`、`relay.rs`、`web.rs` |
| **P1–P3 调试体验** | 右键一键「本地映射」/「填入断点」、独立「构造请求」（含 cURL 粘贴）、`query` 改参、Body 卡片内搜索与 HTML/Markdown 预览、本地回环进程名归属（`lsof +c 0` / `/proc`）、8 槽位 FIFO 断点队列、备注与「清空未星标」、并发批量重放、列表统计与瀑布/时序/依赖图 | `local_proc.rs`、`rules.rs`、`present.rs`、`ui/` |
| **P4 差异化增强** | 连接阶段耗时分段条（DNS/TCP/TLS/Wait/Download）与复用标记、内置 JSON-RPC 2.0 MCP（10 个 Tools：`POST /mcp` 与 `--mcp`）、Rust/Go 代码与类型生成、`no-cache`/`block`/`resolve` 规则、iOS 模拟器一键配置与 Linux 系统证书指引、`Split` 可折叠分面侧栏、JSON 树形视图、单行规则辅助条、全局命令面板与多屏窗口记忆 | `mcp.rs`、`ui/json_tree.rs`、`ui/rule_assist.rs`、`ui/command_palette.rs`、`ui/facet_sidebar.rs` |
| **P5 高阶扩展** | 零配置接入三件套（免证书 SPKI 独立 Chromium / 预注入代理终端 / Android `adb` 一键反代）、`{{...}}` 动态模板变量与 `HMAC-SHA256`、本地回环 HTTP `hook`（`mutate` + `preview`）、零依赖 `.proto` Schema 挂载解码、OpenAPI 3.0.3（JSON/YAML/复制）与批量本地 Mock 目录生成 | `launcher.rs`、`rule_vars.rs`、`webhook.rs`、`proto_schema.rs`、`openapi.rs`、`mock_gen.rs` |
| **v0.4 迁移、接入与安全收口** | 无流量引导卡与底栏「接入中心」、Whistle `.txt` 与 Charles `.xml`/`.chlsj` 迁移导入、并行多组规则（`[[rule_groups]]`）、`socks5://`/`socks5h://` 上游、动态 `/proxy.pac`、LLM/AI SSE 聚合视图（OpenAI/Anthropic/Gemini）、持久化 20 条构造历史、外部根证书探测复用（Whistle/mitmproxy/mkcert/Proxyman）、代理域名准入控制（`allow_hosts` + Docker 严格全拒）、`TopBottom` 宽屏左右并排详情与 Windows ARM64 构建更新 | `import_whistle.rs`、`import_charles.rs`、`proxy/socks5.rs`、`llm_stream.rs`、`external_ca.rs`、`allow_hosts.rs`、`ui/onboarding_card.rs`、`ui/connect_hub.rs`、`ui/rule_groups.rs`、`ui/llm_view.rs`、`ui/access_popover.rs` |
