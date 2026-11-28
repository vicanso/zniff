# zniff 产品与架构问题决策记录（Open Questions）

> 返回 [文档索引](./README.md) · 常规行为边界另见 [已知限制与行为边界](known-issues.md) · v0.5 规划见 [易用性与功能增强规划](usability-roadmap-v0.5.md)

本文档记录涉及**用户习惯、协议语义边界与产品复杂度取舍**的问题。第 1–23 项均已决策并落地（E7、E9 明确暂缓），第 24 项已记录纳入下一版本规划。

---

## 一、界面交互与体验取舍

### 1. 双层过滤（顶栏持久化 `filter` vs 左栏临时 `list_filter`）与「只看 / 屏蔽 / 分面」归属 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/ui/filter_tokens.rs`、`app/src/ui/facet_drawer.rs`、`app/src/ui/facet_sidebar.rs`
- **评估选项**：
  - 选项 A：维持双层职责分离，不提供快捷合并按钮。
  - **选项 B（已采用）**：维持左栏搜索框、分面胶囊与「只看 / 屏蔽」默认写入会话级临时过滤词 `list_filter`；当 `list_filter` 非空时，在左栏搜索框右侧显示紧凑「并入主过滤」（`Merge`）按钮，一键将当前二次筛选词去重并入顶栏主过滤 `profiles.filter`、清空 `list_filter` 并触发配置保存。
  - 选项 C：将 `mute:` 直接写入主过滤，其余保持临时。
- **落地说明**：新增 `merge_secondary_filter` 纯函数与左栏搜索条旁「并入主过滤」动作按钮，既保留左栏即用即弃的轻量临时下钻体验，又支持随时将满意的分面/屏蔽组合一键沉淀进 `SavedProfile`。

### 2. 正文内搜索（`req_search` / `res_search`）的跨流保留策略与 256 KB 分页检索范围 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/ui/detail_body.rs`
- **评估选项**：
  - **选项 A（已采用）**：保留 `req_search` / `res_search` 跨流切换不清空以利连续比对同类接口；当请求或响应正文存在多页（`> 256 KB` 的 `TextWindow` 分页）且搜索词非空时，在正文卡片标题栏命中计数旁显式展示「当前页命中」（`Page matches`）范围角标。
  - 选项 B：跨全部 1 MB 分页做全局检索。
  - 选项 C：切换流量时自动清空 `req_search` / `res_search`。
- **落地说明**：`body_search_box` 感知当前 `TextWindow` 是否存在前/后分页，多页检索时展示「当前页命中」提示，兼顾大文本渲染帧率与语义清晰度。

### 3. 「构造请求 / 编辑重发」键盘提交快捷键 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/ui/compose_box.rs`、`app/src/ui/app_shell.rs`
- **评估选项**：
  - **选项 A（已采用）**：在 `compose_box` 作用域支持 `Cmd/Ctrl+Enter`（`mod+enter`）一键触发发送或断点放行；单行 URL 输入框绑定 `Enter`（`on_submit`）提交发送，且当输入以 `curl` 开头时仅解析填充命令而不误触发送。
  - 选项 B：仅允许点击右上角按钮提交。
- **落地说明**：抽取共用 `submit_compose` 动作并接入按钮点击、`compose_box` 内 `mod+enter`（`When::Always` 穿透多行 `editor`）以及 URL 单行框 `on_submit`（排除 `curl` 粘贴态）。

### 4. 左栏行右键菜单（`row_context_menu`）与详情「更多」菜单的高频项边界 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/ui/flow_table.rs`、`app/src/ui/detail_actions.rs`、`AGENTS.md`
- **评估选项**：
  - 选项 A：维持原 11 项右键菜单不含「编辑重发」。
  - **选项 B（已采用）**：在左栏 `row_context_menu` 的「重放 / 批量重放」下方补入「编辑重发」（`Edit & resend`；若该行正处于请求/响应断点队首则动态显示「编辑请求」/「编辑响应」，中继流量置灰禁用），点击即选中该行并展开右栏 `compose_box`；「另存响应」仍留在详情菜单。
- **落地说明**：左栏行右键菜单与详情「更多」菜单共用 `edit_action_label`，一键打通「右键改参重发 / 改断点报文」路径。

### 5. 规则快速添加面板对 `op=<GraphQL操作名>` 的 UI 覆盖范围 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/rules.rs`、`app/src/ui/quick_rules.rs`、`app/src/ui/filter_panel.rs`
- **评估选项**：
  - 选项 A：仅在少数输入框较少的规则标签提示手写 `op=`。
  - **选项 B（已采用）**：在快速添加区的 `域名 + Path` 匹配行右侧提供紧凑可选 `GraphQL op` 输入框，统一覆盖全部支持 `op=` 的快捷规则（当前共 12 项：`替换`、`转发`、`文件`、`延迟`、`断点`、`响应断点`、`改头`、`改参`、`JSON`、`禁缓存`、`阻断` 以及后续新增的 `Hook`；不支持 `op=` 的 `跨域`、`限速`、`证书`、`解析` 自动隐藏）。
- **落地说明**：`rules.rs` 新增 `QuickRule::supports_graphql_op` 与 `append_quick_rule_with_op`（保留 `append_quick_rule` 兼容委托），`quick_rules.rs` / `filter_panel.rs` 将 `op_pattern` 完整接入添加校验、预设未保存改动检测、配置导出与新建配置重置，并精简 `from`/`to` 冗余占位提示。

### 6. 时序图面板与主列表联动及依赖图源码复制 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/ui/seq_panel.rs`、`app/src/ui/seq_dep_waterfall.rs`、`app/src/ui/seq_swimlane.rs`
- **评估选项**：
  - **选项 A（已采用）**：点击「瀑布图 / 时序图 / 依赖图」预览中的任意请求行直接选中对应 `flow.id` 并关闭时序图面板与编辑框；「源码」标签提供 `sequenceDiagram` 与 `flowchart TD` 切换（从「依赖图」切入源码时默认选中 `flowchart TD`），且处于「依赖图」或 `flowchart TD` 源码模式时「复制 Mermaid / 复制 Markdown」复制 `flowchart_mermaid`。
  - 选项 B：保持纯只读预览与单一 `sequenceDiagram` 导出。
- **落地说明**：打通时序图/瀑布图/依赖图到主列表详情的单点跳转闭环，并完整释放 `SequenceDiagramData.flowchart_mermaid` 的查看与复制能力。

### 7. WebSocket / SSE 消息列表单帧复制按钮 — ✅ 已决策并落地（选项 A）

- **代码位置**：`app/src/ui/detail_headers_msgs.rs`
- **评估选项**：
  - **选项 A（已采用）**：在「消息」卡片每条消息行右侧提供紧凑单帧复制图标按钮，并在卡片标题栏提供「复制全部」（`Copy all`）按钮一键复制当前过滤条件下的全部消息帧。
  - 选项 B：仅靠手动框选复制。
- **落地说明**：单帧复制直接写入该帧载荷文本，「复制全部」按 `[name]\nvalue` 分段拼接当前筛选后的消息帧并在状态栏提示已复制。

---

## 二、Web 操作台、协作脱敏、导入导出与升级策略

### 8. `zniff --web` Wasm 端单流完整下载与内置应急页定位 — ✅ 已决策并落地（选项 B 单流下载 + 选项 A 应急页定位）

- **代码位置**：`app/src/web.rs`、`app/src/ui/detail_actions.rs`
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

- **代码位置**：`app/src/redact.rs`、`app/src/share.rs`、`app/src/config.rs`、`app/src/ui/redact_preview.rs`、`app/src/web.rs`、`app/src/hub_web.rs`
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

- **代码位置**：`app/src/proxy/`、`app/src/session.rs`
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

- **代码位置**：`app/src/proxy/replay.rs`
- **评估选项**：
  - 选项 A：固定使用 `http/1.1` ALPN 重放。
  - **选项 B（已采用）**：当源流量协议为 `Protocol::Http2` 或独立构造 HTTPS 请求时，重放拨号宣告 `["h2", "http/1.1"]` ALPN；若源站协商选定 `h2`，则通过独立新建的单次 `hyper::client::conn::http2` 连接发送请求（不进入共享连接池），源站选定 `http/1.1` 时自动回退 HTTP/1.1。
- **落地说明**：补齐纯 HTTP/2 网关与 gRPC-Web 端点的重放与批量重放兼容性，同时保留每次重放连接隔离的确定性。

### 16. Chunked 流式报文在 `op=` GraphQL 匹配与 `replace` / `json` 改写中的协议对齐 — ✅ 已决策并落地（选项 B）

- **代码位置**：`app/src/proxy/plain_http.rs`、`app/src/proxy/hyper_forward.rs`、`docs/known-issues.md`、`AGENTS.md`
- **评估选项**：
  - 选项 A：仅补齐明文 `op=` 读取与跳过备注，不改写 chunked 报文。
  - **选项 B（已采用）**：明文 HTTP 路径对齐 HTTPS 支持有界读取 `<= 1 MB` 流式请求体以提取 `op=<GraphQL操作名>`；当请求或响应明确命中至少一条 `replace` 或 `json` 规则时，对无 `Content-Length` 的 `chunked` 报文按需有界缓冲最多 `1 MB`（`BODY_CAP`）完成解压、改写并重算 `Content-Length`；未命中改写规则或超出 `1 MB` 的流量保持流式转发（超出上限时在备注记录跳过原因）。
- **落地说明**：在不牺牲普通流量零缓冲转发性能的前提下，彻底解决实际联调中大量网关返回 `Transfer-Encoding: chunked` 小体积 JSON 无法被 `replace` / `json` 改写的痛点。

---

## 四、v0.5 规划决策（已决策并落地）

> 来源：[易用性与功能增强规划](usability-roadmap-v0.5.md)。第 17–23 项已结合代码与 `nsh-core 0.6.1` API 完成评估并确认结论，后续随对应版本落地时补充落地说明。

### 17. 「过滤」与「规则」是否拆成两个入口 — ✅ 已决策并落地（选项 B，v0.5）

- **代码位置**：`app/src/ui/filter_panel.rs`、`app/src/ui/rules_panel.rs`、`app/src/ui/toolbar.rs`、`app/src/ui/profile_dock.rs`
- **问题**：一个弹出层同时承载视图过滤、直通、规则、规则组、上游与配置存取，新用户分不清「看什么」和「改什么」。
- **评估选项**：
  - 选项 A：维持现状，只在弹出层内加分区标题与说明。
  - **选项 B（已采用）**：拆成「过滤」和「规则」两个顶栏入口，配置预设（`SavedProfile`）仍同时保存两者；顶栏分别显示过滤胶囊与 `规则 N 条 · M 组`，「新建配置 / 保存配置」统一收口在预设胶囊入口中，避免双弹出层重复绘制保存栏。
  - 选项 C：右栏增加常驻「规则」页签，代替弹出层。
- **落地说明**（`179575b`）：`filter_panel.rs` 精简为纯视图过滤弹出层；新增 `app/src/ui/rules_panel.rs` 承载独立「规则」弹出层（规则组、编辑器、行辅助条、试跑预览、快速添加、TLS 直通与上游代理）以及顶栏「预设」胶囊（统一收口 `SavedProfile` 切换、保存、导入与导出），顶栏独立展示 `过滤: …` 开关胶囊与 `规则 N 条 · M 组` 角标。

### 18. 是否新增独立「设置」窗口并迁移低频项 — ✅ 已决策并落地（选项 B，v0.5）

- **代码位置**：`app/src/ui/settings_window.rs`、`app/src/ui/settings_tabs.rs`、`app/src/ui/more_menu.rs`、`app/src/ui/more_menu_rows.rs`
- **问题**：设置分散在「更多」、底栏「安全」、底栏「接入」三处，「更多」菜单过长。
- **评估选项**：
  - 选项 A：维持现状，只对「更多」做分组。
  - **选项 B（已采用）**：新增分页「设置」窗口（`Cmd/Ctrl+,`），「接入中心」只保留接入动作；「更多」只保留导出、工具、帮助三组。
- **落地说明**（`d481fac`）：新增 `settings_window.rs` 与 `settings_tabs.rs` 提供 6 个分页（通用 / 证书 / 网络与上游 / 安全 / 中继 / 更新），桌面端采用 `nsh_core::element::desktop::window()` 独立子窗口、Wasm 端采用模态层并绑定 `Cmd/Ctrl+,`；「更多」菜单瘦身为导出 / 工具 / 帮助三组（每组 ≤ 6 项）。

### 19. 是否在每条流量上记录命中的规则 — ✅ 已决策并落地（选项 B，v0.5）

- **代码位置**：`app/src/session.rs`（`Flow`、`RuleHit`）、`app/src/proxy/hub.rs`、`app/src/rules.rs`、`app/src/ui/flow_cells.rs`、`app/src/ui/rule_badges.rs`、`app/src/ui/rule_assist.rs`
- **问题**：当前 `Flow.rule_hit: String` 仅记录空格分隔的动词并在详情摘要展示，无法在列表中直观标识，也无法定位到具体规则分组/行号或统计各规则命中次数。
- **评估选项**：
  - 选项 A：维持仅记录动词，不记录行号。
  - **选项 B（已采用）**：保留 `Flow.rule_hit` 兼容现有 Web/分享/测试，补充轻量命中明细数组（上限 8 条，记录 `group_idx / line_no / verb`，未命中规则的流量零堆分配）；列表行增加紧凑标记、规则编辑器显示命中计数并支持 `rule:` 过滤，HAR 导出写入 `_zniffRules`，MCP `get_flow_detail` 同步返回。
- **落地说明**（`0f780e0`、`7789cf5`）：在 `Flow` 上保留 `rule_hit` 并新增 `rule_hits: Vec<RuleHit>`（上限 8 条，未命中零分配），列表中展示紧凑规则图标，详情摘要在 `rule_badges.rs` 渲染可点击跳转编辑器的规则徽章，同步输出至 HAR `_zniffRules`、MCP `get_flow_detail` 与 `/api/flows`；`rule_assist.rs` 展示规则行本会话命中次数（0 次置灰、点击按 `rule:<行号>` 过滤），`session.rs` 与 `filter_autocomplete.rs` 支持 `rule:` / `-rule:` 过滤与自动补全。

### 20. `Cmd/Ctrl+K` 清空会话的误触保护 — ✅ 已决策并落地（选项 A，v0.4.x）

- **评估选项**：
  - **选项 A（已采用）**：保留 `Cmd/Ctrl+K` 清空会话快捷键（与现有 `Cmd/Ctrl+Shift+P` 命令面板保持兼容），清空后将原流量暂存至 5 秒撤销槽并在界面展示「已清空 N 条 · 撤销」Toast，超时自动释放。
  - 选项 B：改为 `Cmd/Ctrl+Shift+K`，`Cmd/Ctrl+K` 改为打开命令面板。
  - 选项 C：A 与 B 同时做。
- **落地说明**（`ec6a899`）：在 `Session` / `Hub`（含 `hub_web.rs` 与 `web.rs` `/api/hub`）新增 5 秒撤销槽（`clear_with_undo` / `undo_clear` / `expire_clear_undo`，支持恢复时与新到达流量无 ID 冲突合并及超时释放内存），并抽离 `app/src/ui/toast_overlay.rs` 提供中英双语 `[撤销 / Undo]` 提示条。

### 21. WebSocket 帧注入与条件断点的语法边界 — ✅ 已决策并落地（仅采纳 E8，v0.5.x；E7 延至 v0.6）

- **决策结论**：
  - **条件断点（E8，已采用）**：扩展 `break` / `break-res` 语法支持紧凑键值 `header=<k:v>` 与 `body=<substr>`（与现有 `op=<gql>` 风格一致，不引入正则，受 `BODY_CAP = 1 MB` 约束）。
  - **WebSocket 帧注入（E7，暂缓至 v0.6）**：需将现有双向字节对拷改造为帧级合并写通道，复杂度较高，不纳入 v0.5。
- **落地说明**（`b811027`）：在 `rules.rs` 新增 `BreakCondition`（`header=<k:v>` / `header=<k>` 与 `body=<substr>` 子串匹配，受 1 MB `BODY_CAP` 约束），接入 `Hub::needs_request_body_buffer` 与 `hyper_forward.rs` / `plain_http.rs` / `upstream_io.rs` 条件放行路径，并在 `quick_rules.rs` 与 `rule_assist.rs` 支持快速添加与回填；E7 维持延至 v0.6。

### 22. 会话快照与多会话标签是否与「重启清空」边界冲突 — ✅ 已决策并落地（仅采纳 E10，v0.5.x；E9 暂缓）

- **问题**：`known-issues.md` 规定会话为内存加临时分片、重启清空，且不做 SQLite。
- **决策结论**：
  - **导入会话只读标签（E10，已采用）**：导入的 `.har` / `.chlsj` 作为只读标签与实时会话并列切换、支持互相对比，不改动现有会话内存与清理语义。
  - **退出自动保存 HAR 快照（E9，暂缓 / 维持现状）**：退出时序列化大体积会话易拖慢关闭且增加敏感流量落盘风险，维持「重启清空 + 手动导出 HAR」的明确边界，无需改动 `known-issues.md`。
- **落地说明**（`b4505d7`）：新增 `app/src/ui/session_tabs.rs` 管理实时会话与最多 8 个只读导入 `.har`/`.chlsj` 标签（仅打开导入标签时展示紧凑标签栏），完整支持跨标签 `BaselineRef` 基准对比与只读写入保护；E9 维持暂缓。

### 23. 菜单栏 / 系统托盘常驻 — ✅ 已决策并落地（轻量版，v0.5.x）

- **技术核对**：`nsh-core 0.6.1` 已原生提供 `desktop::tray()`、`desktop::set_tray()` 与 `main_window().on_close_press(...)`，无需另写平台实现。
- **决策结论**：采用轻量托盘菜单（显示窗口、开始/停止、系统代理开关、退出）；「关闭窗口时最小化到托盘」默认关闭（维持关窗即停抓包的安全默认，在设置窗口中按需开启）。
- **落地说明**（`d09f48e`）：新增 `app/src/ui/tray.rs` 实现桌面端响应式双语系统托盘菜单与 `on_close_press` 隐藏至托盘生命周期，并在 `config.rs` 与设置「通用」页新增默认关闭的 `minimize_to_tray` 开关（wasm32 编译无侵入）。

---

## 五、下一版本规划待办

### 24. 脚本勾子（`Script Hooks`）界面点击入口与内置 TSX 编辑面板 — 📋 已记录待下一版本实现

- **关联模块**：`app/src/hook_spec.rs`、`app/src/hook_runtime.rs`、`app/src/ui/command_palette.rs`、`app/src/ui/hook_run_modal.rs`、`app/src/ui/rules_panel.rs`、`app/src/ui/more_menu.rs`（交互与编辑器可复用现有自定义详情布局管理面板 `app/src/ui/layout_settings.rs`、`app/src/ui/layout_settings_editor.rs`）
- **当前现状**：
  - 脚本文件存放在 `<data_dir>/hooks/<id>.tsx`（或 `.ts`），目前依赖外部编辑器编写；
  - 初始化示例脚本（`init_script_hooks`）与触发 `onRun` 主动勾子（`run_script_hook`）仅提供 `Cmd/Ctrl+Shift+P` 全局命令面板入口，顶栏「更多」菜单与「规则」弹出层暂无鼠标点击入口，也尚未提供应用内 TSX 代码与 `[[script_hooks]]` 可视化管理编辑面板。
- **下一版本规划方案**：
  1. **可点击界面入口**：在顶栏「规则」弹出层（新增「脚本勾子 / Hooks」管理入口或页签）及「更多 → 工具」菜单补齐鼠标可点击入口（同时在「更多 → 工具」补上打开全局命令面板 `Cmd/Ctrl+Shift+P` 的菜单项）。
  2. **内置 TSX 编辑与配置管理面板**：复用 `nsh_base::editor()` 与 `zniff-script-worker` 编译/导出校验能力（对齐 `layout_settings_editor.rs` 体验），支持在应用内直接新建/重命名/编辑/删除 `<data_dir>/hooks/*.tsx`，可视化勾选 `stages`（`request` / `response` / `preview` / `run`）、配置 `match` 匹配模式、`timeout_ms`、`vars` 键值变量与 `enabled` 开关，展示熔断状态并支持一键重置熔断与试跑 `onRun`。
  3. **Web 操作台对齐**：在 `zniff --web`（`web.rs` / `hub_web.rs`）同步评估补齐受 `web_auth` 保护的脚本列表与编辑接口，或显式维持服务端只读安全边界。
