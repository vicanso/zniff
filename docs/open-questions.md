# zniff 产品与架构问题决策记录（Open Questions — 全部已决策并落地）

本文档记录代码审计中涉及**用户习惯、协议语义边界与产品复杂度取舍**的 16 项问题的评估、确认结论与落地实现。常规边界说明另见 [docs/known-issues.md](known-issues.md)。

---

## 一、界面交互与体验取舍

### 1. 双层过滤（顶栏持久化 `filter` vs 左栏临时 `list_filter`）与「只看 / 屏蔽 / 分面」归属 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/lib.rs`
- **评估选项**：
  - 选项 A：维持双层职责分离，不提供快捷合并按钮。
  - **选项 B（已采用）**：维持左栏搜索框、分面胶囊与「只看 / 屏蔽」默认写入会话级临时过滤词 `list_filter`；当 `list_filter` 非空时，在左栏搜索框右侧显示紧凑「并入主过滤」（`Merge`）按钮，一键将当前二次筛选词去重并入顶栏主过滤 `profiles.filter`、清空 `list_filter` 并触发配置保存。
  - 选项 C：将 `mute:` 直接写入主过滤，其余保持临时。
- **落地说明**：新增 `merge_secondary_filter` 纯函数与左栏搜索条旁「并入主过滤」动作按钮，既保留左栏即用即弃的轻量临时下钻体验，又支持随时将满意的分面/屏蔽组合一键沉淀进 `SavedProfile`。

### 2. 正文内搜索（`req_search` / `res_search`）的跨流保留策略与 256 KB 分页检索范围 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/lib.rs`
- **评估选项**：
  - **选项 A（已采用）**：保留 `req_search` / `res_search` 跨流切换不清空以利连续比对同类接口；当请求或响应正文存在多页（`> 256 KB` 的 `TextWindow` 分页）且搜索词非空时，在正文卡片标题栏命中计数旁显式展示「当前页命中」（`Page matches`）范围角标。
  - 选项 B：跨全部 1 MB 分页做全局检索。
  - 选项 C：切换流量时自动清空 `req_search` / `res_search`。
- **落地说明**：`body_search_box` 感知当前 `TextWindow` 是否存在前/后分页，多页检索时展示「当前页命中」提示，兼顾大文本渲染帧率与语义清晰度。

### 3. 「构造请求 / 编辑重发」键盘提交快捷键 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/lib.rs`
- **评估选项**：
  - **选项 A（已采用）**：在 `compose_box` 作用域支持 `Cmd/Ctrl+Enter`（`mod+enter`）一键触发发送或断点放行；单行 URL 输入框绑定 `Enter`（`on_submit`）提交发送，且当输入以 `curl` 开头时仅解析填充命令而不误触发送。
  - 选项 B：仅允许点击右上角按钮提交。
- **落地说明**：抽取共用 `submit_compose` 动作并接入按钮点击、`compose_box` 内 `mod+enter`（`When::Always` 穿透多行 `editor`）以及 URL 单行框 `on_submit`（排除 `curl` 粘贴态）。

### 4. 左栏行右键菜单（`row_context_menu`）与详情「更多」菜单的高频项边界 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/lib.rs`、`AGENTS.md`
- **评估选项**：
  - 选项 A：维持原 11 项右键菜单不含「编辑重发」。
  - **选项 B（已采用）**：在左栏 `row_context_menu` 的「重放 / 批量重放」下方补入「编辑重发」（`Edit & resend`；若该行正处于请求/响应断点队首则动态显示「编辑请求」/「编辑响应」，中继流量置灰禁用），点击即选中该行并展开右栏 `compose_box`；「另存响应」仍留在详情菜单。
- **落地说明**：左栏行右键菜单与详情「更多」菜单共用 `edit_action_label`，一键打通「右键改参重发 / 改断点报文」路径。

### 5. 规则快速添加面板对 `op=<GraphQL操作名>` 的 UI 覆盖范围 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/rules.rs`、`app/src/lib.rs`
- **评估选项**：
  - 选项 A：仅在少数输入框较少的规则标签提示手写 `op=`。
  - **选项 B（已采用）**：在快速添加区的 `域名 + Path` 匹配行右侧提供紧凑可选 `GraphQL op` 输入框，统一覆盖全部 11 种支持 `op=` 的规则（`替换`、`转发`、`文件`、`延迟`、`断点`、`响应断点`、`改头`、`改参`、`JSON`、`禁缓存`、`阻断`；不支持 `op=` 的 `跨域`、`限速`、`证书` 自动隐藏）。
- **落地说明**：`rules.rs` 新增 `QuickRule::supports_graphql_op` 与 `append_quick_rule_with_op`（保留 `append_quick_rule` 兼容委托），`lib.rs` 将 `op_pattern` 完整接入添加校验、预设未保存改动检测、配置导出与新建配置重置，并精简 `from`/`to` 冗余占位提示。

### 6. 时序图面板与主列表联动及依赖图源码复制 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/lib.rs`
- **评估选项**：
  - **选项 A（已采用）**：点击「瀑布图 / 时序图 / 依赖图」预览中的任意请求行直接选中对应 `flow.id` 并关闭时序图面板与编辑框；「源码」标签提供 `sequenceDiagram` 与 `flowchart TD` 切换（从「依赖图」切入源码时默认选中 `flowchart TD`），且处于「依赖图」或 `flowchart TD` 源码模式时「复制 Mermaid / 复制 Markdown」复制 `flowchart_mermaid`。
  - 选项 B：保持纯只读预览与单一 `sequenceDiagram` 导出。
- **落地说明**：打通时序图/瀑布图/依赖图到主列表详情的单点跳转闭环，并完整释放 `SequenceDiagramData.flowchart_mermaid` 的查看与复制能力。

### 7. WebSocket / SSE 消息列表单帧复制按钮 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/lib.rs`
- **评估选项**：
  - **选项 A（已采用）**：在「消息」卡片每条消息行右侧提供紧凑单帧复制图标按钮，并在卡片标题栏提供「复制全部」（`Copy all`）按钮一键复制当前过滤条件下的全部消息帧。
  - 选项 B：仅靠手动框选复制。
- **落地说明**：单帧复制直接写入该帧载荷文本，「复制全部」按 `[name]\nvalue` 分段拼接当前筛选后的消息帧并在状态栏提示已复制。

---

## 二、Web 操作台、协作脱敏、导入导出与升级策略

### 8. `zniff --web` Wasm 端单流完整下载与内置应急页定位 — ✅ 已决策并落地（选项 B 单流下载 + 选项 A 应急页定位）

- **代码位置**：`app/src/web.rs`、`app/src/lib.rs`
- **评估选项**：
  - 内置无 Wasm 产物时的 HTML 页面保持 **选项 A** 最小应急操作台定位，避免维护两套复杂前端；
  - 单条响应下载采用 **选项 B**：在 `web.rs` 新增 `GET /api/flows/{id}/body` 原文附件下载路由（自动从 `zniff-spill` 分片按需还原完整响应体、回填响应 `Content-Type` 与推荐文件扩展名，并遵守当前 `redact_export` / `redact_strict` 脱敏配置），同时在 Wasm 端详情「更多」菜单开放「另存响应…」（`Save response…`）。
- **落地说明**：前后端与往返测试均已就位，浏览器操作台可直接下载单条二进制或大体积响应原文。

### 9. macOS 自动升级在缺失 `SHA256SUMS` 时的校验策略 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/updater.rs`
- **评估选项**：
  - **选项 A（已采用）**：全平台统一强制要求 `SHA256SUMS`——macOS 与 Linux / Windows 一致，若 Release 资产缺失 `SHA256SUMS` 或未找到匹配安装包哈希条目则直接拒绝升级安装。
  - 选项 B：对已签名 `.app` 允许无 `SHA256SUMS` 回退。
- **落地说明**：统一全平台自动更新供应链完整性校验基线，消除缺失哈希清单放行分支。

### 10. `cURL` 粘贴解析的扩展参数边界（`--json`、`-G`、`-I`、多段 `-d`） — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/present.rs`、`docs/known-issues.md`
- **评估选项**：
  - 选项 A：维持原有参数子集。
  - **选项 B（已采用）**：扩展支持 `--json <data>`（自动补充 `Content-Type: application/json`、`Accept: application/json` 及默认 `POST`）、`-I` / `--head`（默认方法设为 `HEAD`）、多段 `-d` / `--data*` 以 `&` 拼接，以及 `-G` / `--get` 将数据拼接到 URL query（仅继续排除读取本地磁盘文件的 `-F @file`）。
- **落地说明**：纯函数解析器已覆盖现代 API 文档常用的 `curl --json`、`-G`、`-I` 与多段 `-d` 写法。

### 11. 导出/分享脱敏的覆盖深度与非结构化正文策略 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/share.rs`、`app/src/config.rs`、`app/src/lib.rs`、`app/src/web.rs`、`app/src/hub_web.rs`
- **评估选项**：
  - 选项 A：对 `multipart/form-data` 与二进制正文仅保留原字节并打标记。
  - **选项 B（已采用）**：新增 `multipart/form-data; boundary=...` 字段级结构化脱敏（当分片 `name` 或 `filename` 命中敏感字段时将分片内容替换为 `[REDACTED]`，对 JSON / `x-www-form-urlencoded` 子分片递归脱敏并按原 `boundary` 重组报文），并新增可选 `redact_strict`「严格遮罩」（`Strict mask`）开关——开启后，任何无法结构化脱敏的非空二进制或非结构化正文均替换为 `[REDACTED: unstructured or binary body omitted in strict mode]`。
- **落地说明**：`config.toml` 持久化 `redact_strict`（默认 `false`），「脱敏预览与设置」弹窗提供「严格遮罩」开关并实时更新 `diff_view` 预览与计数，完整同步至 HAR/HTML 导出、`/api/flows/{id}/body` 及实时分享链接（含 `@strict` 标记同步）。

### 12. HAR 1.2 导入导出的 `_webSocketMessages` 扩展字段兼容性 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/har.rs`
- **评估选项**：
  - **选项 A（已采用）**：HAR 导出时为非空 `Flow.messages` 写出 Chrome DevTools / Proxyman / Charles 通用的 `_webSocketMessages` 数组（`type: "send"|"receive"`、`time`、`opcode`、`data`），HAR 导入时若存在 `_webSocketMessages` 则还原回 `Flow.messages`。
  - 选项 B：仅在 HTML 快照保留 WebSocket 消息。
- **落地说明**：符合 HAR 1.2 `_` 自定义扩展规范，实现与主流抓包工具及 DevTools 的 WebSocket 帧无损互导。

---

## 三、代理内核、过滤语法与规则语义

### 13. 断点「丢弃」（`BreakDecision { drop: true }`）的客户端响应语义与统计口径一致性 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/proxy.rs`、`app/src/session.rs`
- **评估选项**：
  - 选项 A：维持返回 `204 No Content` 并单独修补统计口径。
  - **选项 B（已采用）**：对齐 `block <pattern> drop` 断连语义——点击断点「丢弃」时直接关闭客户端连接或重置流，不返回伪造的 HTTP 状态码（列表状态为 `ERR`，原因为 `断点丢弃`）。
- **落地说明**：客户端能够真实感知请求被中止而非误判为 `2xx` 成功，同时天然统一 `FilteredStats`、`失败` 过滤词与分面抽屉 `SessionFacets` 三处统计口径。

### 14. 带冒号的负向过滤词（如 `-host:x`、`-path:/health`、`-status:204`）处理方式 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/session.rs`
- **评估选项**：
  - **选项 A（已采用）**：全面支持字段级负向前缀——解析 `-host:` / `-domain:`、`-path:`、`-app:` / `-proc:` / `-ip:` / `-client:`、`-status:`、`-method:`、`-type:` / `-ct:`、`-header:`、`-body:`、`-note:` / `-comment:`、`-op:` / `-gql:` / `-operation:` 等，对对应结构化子条件取反匹配。
  - 选项 B：仅将 `-host:` / `-path:` 映射为 `mute:`。
- **落地说明**：消除用户在过滤框输入 `-host:cdn.com` 或 `-status:204` 时被静默忽略的陷阱，支持任意字段级精确排除。

### 15. 重放 / 批量重放对纯 HTTP/2 源站的 ALPN 协商兼容性 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/proxy.rs`
- **评估选项**：
  - 选项 A：固定使用 `http/1.1` ALPN 重放。
  - **选项 B（已采用）**：当源流量协议为 `Protocol::Http2` 或独立构造 HTTPS 请求时，重放拨号宣告 `["h2", "http/1.1"]` ALPN；若源站协商选定 `h2`，则通过独立新建的单次 `hyper::client::conn::http2` 连接发送请求（不进入共享连接池），源站选定 `http/1.1` 时自动回退 HTTP/1.1。
- **落地说明**：补齐纯 HTTP/2 网关与 gRPC-Web 端点的重放与批量重放兼容性，同时保留每次重放连接隔离的确定性。

### 16. Chunked 流式报文在 `op=` GraphQL 匹配与 `replace` / `json` 改写中的协议对齐 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/proxy.rs`、`docs/known-issues.md`、`AGENTS.md`
- **评估选项**：
  - 选项 A：仅补齐明文 `op=` 读取与跳过备注，不改写 chunked 报文。
  - **选项 B（已采用）**：明文 HTTP 路径对齐 HTTPS 支持有界读取 `<= 1 MB` 流式请求体以提取 `op=<GraphQL操作名>`；当请求或响应明确命中至少一条 `replace` 或 `json` 规则时，对无 `Content-Length` 的 `chunked` 报文按需有界缓冲最多 `1 MB`（`BODY_CAP`）完成解压、改写并重算 `Content-Length`；未命中改写规则或超出 `1 MB` 的流量保持流式转发（超出上限时在备注记录跳过原因）。
- **落地说明**：在不牺牲普通流量零缓冲转发性能的前提下，彻底解决实际联调中大量网关返回 `Transfer-Encoding: chunked` 小体积 JSON 无法被 `replace` / `json` 改写的痛点。
