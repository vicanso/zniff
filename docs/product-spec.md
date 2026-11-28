# zniff 产品与功能详细规格说明

本文档承接原 `AGENTS.md` 中沉淀的产品细节与交互规格，供按需查阅。日常开发与架构红线以根目录 [`AGENTS.md`](../AGENTS.md) 与 `.agents/rules/` 为准；已知缺口见 [`known-issues.md`](./known-issues.md)，开放权衡见 [`open-questions.md`](./open-questions.md)。

---

## 1. 界面布局、列表列、过滤与快捷键

- **双布局模式**：支持「左右分栏（`Split`，默认）」与「上下分栏（`TopBottom`，宽屏详情将请求卡片排在左侧、响应卡片排在右侧）」。
  - `Split` 左侧默认展示「域名 / 应用分面」侧栏：右缘可拖宽 `176–340`，标题栏提供收起/展开按钮并记在 `config.toml` 的 `sidebar_collapsed`；当用户手动收起或窗口宽 `< 880px` 自动收起时，切换回列表顶部的快捷搜索与可折叠分面抽屉（支持根域名子串聚合、整组过滤/屏蔽、错误数红标）。
  - 左栏列表默认宽 `660`（`Split` 下随窗口宽与侧栏当前显隐宽度实时约束以保留详情区至少 `396px` 空间），松开后记在 `config.toml` 的 `list_width`；`TopBottom` 上半高记在 `list_height`；分割条悬停高亮。
- **列模型、排序与展示**：
  - 默认列：`序号、方法、路径、状态、间隔、耗时、大小`，按到达时间从上到下排列，最新在底部（点击表头在 `↑` 升序 / `↓` 降序 / 默认顺序间切换）。
  - 可选隐藏列：`协议`、`主机`、`来源 IP`（本机回环异步识别进程名并记为 `127.0.0.1 (进程名)`）。`show_id`、`show_method`、`show_path`、`show_status`、`show_gap`、`show_elapsed`、`show_size` 默认开；`show_protocol`、`show_host`、`show_client_ip` 默认关。
  - 列顺序支持直接拖拽表头或在列菜单上下调整（固定区「序号、方法、路径」区域内调序，滚动区其余列区域内调序，记入 `column_order`，菜单内可一键恢复默认）；`column_widths` 停半秒记住（路径默认 `300`），藏起的列不占宽度。
  - 间隔列显示距可见列表上一行的间隔（首条 `0 ms`，过滤后按可见行重算），悬停显示本地 `HH:MM:SS.mmm`（与详情摘要绝对时间一致）；耗时列从请求开始到收完响应（未完显示 `—`，`>= 1s` 用警告色）。
  - 列菜单底部可切换路径展示模式（`path_display`：`自动` / `仅展示 Path` / `域名 + Path` / `完整 URL`，兼容旧 `show_full_url`）与省略位置（`ellipsize`：`head` / `middle` / `tail`，默认 `middle`），悬停显示完整全文。
  - 列表滚动距离底部不超过一行时自动跟随新流量，向上翻滚即暂停跟随。
- **过滤语法与双过滤**：
  - 过滤框常驻（停半秒写入 `config.toml` 的 `filter`），词间 AND、词内 `|` 或 `,` 表示 OR：支持 `host:` / `domain:`（含 `*.` 通配）、`path:`、`ip:` / `client:` / `app:` / `proc:`、方法精确、状态精确或 `1xx`–`5xx`、`失败`、`type:` / `ct:`、`body:`（搜请求/响应体与 WS 消息）、`header:`（头名与头值）、`note:` / `comment:`、`repeat:` / `repeat:<id>`、`op:` / `gql:` / `operation:`（GraphQL 操作名，含 `anonymous` 与批量数组；详情摘要带 `gql:<操作名>` 徽章）、`pin:`、`mute:`、`slow:`（`>= 1s`）、`-词` 及各类 `-前缀:` 否定词；其余匹配主机、路径或备注。
  - 左栏顶部搜索框用于实时过滤（存在生效主过滤时进入「二次过滤」，右侧提供「并入主过滤」一键去重合并并保存；无主过滤时提供「设为主过滤」固定进顶栏主过滤）。
- **右键菜单、快捷键与命令面板**：
  - 行右键菜单使用临时 `picked` 行跟踪（不暗改当前选中流或清掉草稿），支持复制 URL / cURL / 给 AI、只看、屏蔽、星标、基准、备注、重放、批量重放、编辑重发（断点队首时为编辑请求/响应）、填入 `break`、本地映射与快速规则（其中 Mock 统一拉起内联 Mock 编辑器）。
  - 快捷键：`↑`/`↓` 切换选中（输入框聚焦时保留光标移动）、`Cmd/Ctrl+K` 清空会话（5 秒可撤销）、`Cmd/Ctrl+F` 聚焦列表过滤框、`Cmd/Ctrl+R` 重放（`make dev` 可能拦截）、`Cmd/Ctrl+Shift+P` 打开命令面板、`Esc` 收起弹窗/编辑框或退回列表统计视图。
- **顶栏「过滤」配置面板与预设**：
  - 右对齐弹出层，`max_h` 随 `window_h - 88` 约束、双栏宽在 `680–1360` 自适应（覆盖左栏约 30% 并保留前 70% 固定列）；左侧为完整过滤与快捷开关，右侧为规则编辑器（`#` 注释 + 单行辅助条）与上游/中继接入；底部一整行固定操作栏含预设下拉、历史、名称、导出 TOML、导入/拖拽、新建配置、保存（严格按脏状态控制禁用/启用）。
  - 选中配置或主过滤后顶栏呈现 `[✓ <配置/条件>]` 一体化胶囊，仅主过滤非空时支持一键暂停/恢复；配置按钮仅在未保存、规则/断点关闭或语法错误时显示提醒点；代理认证与 `allow_hosts` 归口底栏「安全」弹出，中继地址归口顶栏「更多 -> 中继设置」。

---

## 2. 列表统计、详情检查器与编辑器

- **列表统计与时序/依赖图**：
  - 未选中流量且列表非空时显示当前列表统计（总请求、总流量、平均耗时、状态分布、最慢请求直达及「生成时序图」）；顶栏「更多」同样提供入口。
  - 支持勾选/全选/反选、「含响应」「含查询参数」开关，生成 Mermaid `sequenceDiagram` 与 `flowchart TD`，提供「瀑布图 / 时序图 / 依赖图 / 源码」四视图（预览行使用 `virtual_list`，>200 条仅预览最近 200 条但复制保留全量）。
- **详情卡片、Body 与消息**：
  - 选中流量后依次为摘要（不折叠）、响应 body、响应头、请求 body（非空或有说明才画）、查询参数、请求头、行级对比、消息。除行级对比外的 6 张内容卡片折叠状态持久化在 `folded_cards`。
  - 失败行状态 `ERR` 并附步骤/猜测原因，进行中 `—`。
  - Body 卡片带常驻字节数徽章、`美化 | 树形 | 原文` 切换（对可解析 JSON 启用 `nsh_base::tree`，默认仅展开根 `$`，支持搜索路径展开与双击复制节点值）、正文分页（UTF-8 单页 256 KiB「往前/往后」、Hex 4 位行号前 4 KiB 16/32 字节自适应）、内容搜索、`editor` 自动换行高度估算（最多 15 行）、图片原尺寸预览（超 200px 等比缩小）及独立 HTML/Markdown「预览」开关。
  - Head / Query / WS/SSE 消息卡片支持单行悬停透明占位复制（1.4s ✓ 本地确认）+「复制全部」+ 方向/帧类型/关键字筛选；Cookie `;` 拆分（`Set-Cookie` 不拆）、头按名称升序美化、原文保序；附带 JWT/时间戳/Base64 解码、5 段耗时拆解条、`diff_view` 行级对比。
- **重放、构造与脱敏分享**：
  - 「批量重放」支持次数（1–100）、并发（1–20）、间隔 ms，结果带 `repeat:<id> (i/N)` 备注。
  - 「构造请求 / 编辑重发」支持 `curl` 命令智能解析粘贴、`Cmd/Ctrl+Enter` 发送、`Enter` 发单行 URL；停在断点时可改请求/响应后「放行」，支持队列点击置顶（`BREAK_QUEUE_CAP = 8`，队首 60s 超时放行）或「丢弃」（`ERR · 断点丢弃`）。
  - 详情「更多」内置 11 种代码/结构体片段导出；顶栏统计胶囊左侧显示 `统计 <筛选数 / 总数>`；顶栏「更多」提供导出 `[HAR][HTML]`、`OpenAPI [JSON][YAML][复制]`、脱敏开关与 `diff_view` 预览、实时分享 `[15m][1h][8h]`、免配 `[浏览器][终端][ADB]`、复制配置 `[代理环境][MCP]`、证书、`检查更新`、`导出诊断包`、`清空未星标`（置灰 + 二次确认）与语言/等宽字体切换。

---

## 3. 证书管理、配色、语言与字体

- **证书与环境注入**：
  - 根 CA 名 `zniff CA`（移除时同时清理旧名 `Snoop CA`）；区分 `NotInstalled` / `InstalledUntrusted` / `Trusted`，自动探测本机外部代理根证书并支持一键复用。
  - macOS 支持 `xcrun simctl` 向全部运行中 `Booted` 模拟器注入证书与代理并在停抓包/退出时恢复；Linux 提供发行版 `trust anchor` / `update-ca-*` 复制命令。
- **配色、双语与字体**：
  - 所有表面通过 `AdaptiveColor::rgb` 分层（背景、Chrome、列表、斑马纹、选中行、卡片、代码框），方法徽章与状态码使用独立语义色。
  - 文案一律使用 `tr(zh, en)`（持有 `Lang` 的非 UI 路径用 `t(lang, zh, en)`）；错误原因经 `localize_error` 展示；`/zniff` 依据 `Accept-Language` 选择中文/英文。
  - 默认打包 JetBrains Mono NL（4 字重，`app/assets/fonts/` zlib 压缩嵌入，无头 `--no-default-features` 不嵌入），通过 `font_mono` token 全局下发；「系统」开关记入 `system_mono_font = true`。

---

## 4. 抓包内核、会话管理、规则与旁路处理

- **传输、TLS 与会话存储**：
  - 不使用 `hudsucker`，直接使用 `hyper` + `rustls` + `tokio`；跟随 ClientHello ALPN；TLS 探测失败关闭原连接并新开 TCP 放行原始 ClientHello，ServerHello 前失败转 TCP 隧道（开启「隧道失败」开关后入列表），ServerHello 后仅报错。
  - 旁路反序解压记录：单条正文上限 1 MB（`BODY_CAP`），未星标 >16 KB（`INLINE_BODY_CAP`）顺序落盘至临时 `zniff-spill/` 环形分片（单分片 64 MB `SPILL_SEGMENT_CAP`，最多 8 片 `SPILL_MAX_SEGMENTS` 共 512 MB，闲置 30m `SPILL_SEGMENT_TTL` 回收，Unix 启动即 unlink 匿名化），点详情/导出/重放/`body:` 时 `pread` 读取。内存上限 256 MB（`SESSION_CAP`），星标最多 32 条（`PIN_CAP`）且不参与淘汰。
  - WebSocket 旁路解析（剥离 `Sec-WebSocket-Extensions`），WS/SSE 消息上限 200 条或 256 KB；清空会话不重置单调递增 `id`。
  - 默认监听 `127.0.0.1:8877`（`listen_port` 禁 `0`），`listen_all` 开启绑 `0.0.0.0`；底栏「扫码」弹框展示网卡选择（优先公网 IPv4，避开隧道与 `198.18.0.0/15`）、二维码与 iOS/Android 同步步骤。
- **准入、直通、上游池与规则引擎**：
  - `bypass` 支持 `*` 与 `only:`；`allow_hosts` 支持 `--strict-allow-hosts` / `ZNIFF_STRICT_ALLOW_HOSTS=1`（Docker 默认开启），优先级 `--allow-hosts` > 非空环境变量 > `config.toml`（CLI/Env 锁定只读，拦截 `CONNECT`/SNI/HTTP/`map-remote`/重放及 `/proxy.pac`，豁免 `/zniff` 等本地路由）。
  - `upstream_proxy` 支持凭据自动注入 `Proxy-Authorization`；HTTP/1 明文 & HTTP/2（每源站最多 16 条缓存，45s 空闲过期）连接池；重放独立建连，支持按源协议或构造请求 ALPN 协商 H2/H1。
  - 终点规则（`map-local`、`map-remote`、`break`、`break-res`、`block`）按首次命中执行；叠加规则（`delay`、`<30s>`、`throttle`、`cert`、`client-cert`、`resolve`/`dns`/`hosts`、`header`、`query`、`cors`、`no-cache`、`replace`、`json`、`hook`、`proto`）可累加；支持 `op=` GraphQL 操作名过滤与 `{{...}}` 动态模板变量（含 `HMAC-SHA256` 等）；支持目录静态 `map-local` + `fallback`、本机回环 `hook`（`mutate`/`preview`）、零依赖 `.proto` 解码与 OpenAPI 3.0.3 导出。
  - **TypeScript / TSX 报文勾子（`[[script_hooks]]` / `hook <host[/path]> @<id>`）**：按需在后台沙箱化 QuickJS 工作池执行 `onRequest` / `onResponse` / `onPreview` / `onRun` 四阶段脚本（支持 `zniff-hook.d.ts` 类型声明、`ctx.query` / `.text()` / `.json()` / `.form()` 与 `query` / `setHeaders` / `json` / `form` 增量改包，以及四阶段纯插入式 `view`——`onRequest`/`onResponse` 经旁路 LRU 插入各自详情区顶部且不隐藏任何原始正文/Query/Header，`onPreview` 支持顶部 `view` 概览与 `req.view`/`res.view` 分侧顶部插入，`onRun` 弹出原生 `dialog()` 报告与 MCP `view` 输出；内置 `14` 个 TSX 声明式组件 `<Section>`/`<KeyValue>`/`<Badge>`/`<Code>`/`<View>`/`<Row>`/`<Col>`/`<Text>`/`<Divider>`/`<Callout>`/`<Table>`/`<Collapsible>`/`<Tabs>`/`<Tab>` 及零 JS 常驻交互 `copyValue`/`selectFlowId`/`setFilter`，`zniff --web` 经 `/api/flows/{id}` 自动同步渲染且无端 `wasm32` JS 引擎增量）；内置冻结对象 `$`（与 `zniff` 同指同一份只读 SDK：`$.flows.*` / `zniff.flows.*` ID-first 读写与 `zniff-spill` 按需 `pread`、`$.codec.*` / `zniff.codec.*` UTF-8 / Base64 / Base64URL / Hex / URL / Form-URL / `gzip` / `deflate` / `zlib` / `zstd` / `brotliDecompress` / `crc32` / `jwtDecode` / `protobufDecodeWire` 与 `$.crypto.*` / `zniff.crypto.*` MD5/SHA-1/256/384/512 及 HMAC `*Hex`、AES-GCM / ChaCha20-Poly1305、AES-CBC PKCS#7、PBKDF2 / HKDF、`timingSafeEqual` / `randomBytes` / `uuidV4`）。

---

## 5. 分享、中继、`zniff --web` 与发布更新

- **本地分享与中继**：
  - 「实时分享」生成 `/s/{uuid}`（仅同时存在一条，支持 `15m`/`1h`/`8h`，`flows.json` 按 `Accept-Encoding` gzip 压缩）。
  - 配置合法 `relay_url` + 单词 `relay_token` 后每秒向中继推流（单份 <= 32 MiB，最长 8h），`zniff --relay`（可配 `--view-auth`）或另一端「接收中继」通过本监听端口 origin-form / exact absolute-form 接收并在列表中标注 `中继 <8位hex>`。只读查看器 HTML 与「导出 HTML」共用同一套前端查看器。
- **Web 操作台与无头部署**：
  - `zniff --web`（默认 `127.0.0.1:8080`）不开窗、不改系统代理，抓包端口仅在操作台点击开始后绑定；通过 `nsh package --target web` 编译 `app/.nsh/web` 提供完整界面；支持 `--relay` 伴生中继端口（默认 `8879`）、`--no-capture` 及 4 端口隔离认证（`web_auth` / `proxy_auth` / `relay_token` / `relay_view_auth`）；Docker 镜像以 `--no-default-features` 构建（默认 `ZNIFF_DATA_DIR=/data`）。
- **系统代理、更新与打包**：
  - 默认不改系统代理；仅 `use_system_proxy` 开启且主动抓包期间改动并在 `proxy-snapshot.toml` 留存现场，异常退出后启动时通过 `restore_if_dirty()` 恢复。
  - 自动更新每 2 天（`AUTO_CHECK_EVERY`）检查 Releases、强制校验 `SHA256SUMS` 并执行 macOS DMG / Linux tar.gz / Windows zip 原地替换重启；macOS 由 `sign_and_package_macos.sh --embed-sysext` 打包 Universal `.app`（含 `NETransparentProxyProvider` System Extension），CLI 由 `make release-cli` 构建。
