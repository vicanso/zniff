# zniff 功能审视与体验优化分析报告

> [!NOTE]
> **历史归档快照（基线 `v0.2.1`，2026-11-18）**：本报告记录的是 `v0.2.1` 阶段的实机审计结果。报告中的 P0 缺陷与体验问题（明文建连错误标 `ERR`、三栏 `WideThree` 挤压统一为 `Split` 可折叠侧栏 + `TopBottom`、统计总流量截断、`lsof +c 0` 应用识别、顶栏过滤胶囊语义等）均已于 `v0.3.0+` 修复，且 `proxy.rs` 已拆分为 `app/src/proxy/`。当前能力与边界请以 [文档索引](./README.md) 和 [已知限制与行为边界](./known-issues.md) 为准。

- **评估日期**：2026-11-18
- **基线版本**：`zniff v0.2.1`（`app/Cargo.toml`，含当前工作区「分栏布局切换（Split / TopBottom / WideThree）」、「应用图标与来源设备识别」、「顶栏过滤启停开关合并与视觉微调」等演进）
- **评估方式**：静态代码审计（仅只读排查 `app/src/`，**零源码改动**） + 隔离数据目录实机运行（`ZNIFF_DEV=1 zniff --data-dir /tmp/zniff-audit-run`，`1200×760` 默认桌面窗口 + Mock HTTP 多协议/多状态流量注入 + Retina 窗口截图实测）
- **实机截图归档**：`docs/screenshots/audit-v0.2/`（4 张 `2400×1520` 原生窗口截图）

---

## 一、实机运行与截图索引

本次测试通过本地 Mock 源站配合 `resolve` 规则注入了覆盖 **GraphQL POST、Query + Set-Cookie、慢请求（≥ 1s）、401/502 异常状态、静态 JS / Markdown、端口拒绝建连失败（`127.0.0.1:18999`）、回环复用连接、不同客户端 `User-Agent`（curl / iPhone / Android Dalvik / macOS CFNetwork）** 的混合流量，分别在三种主工作区布局下进行了实机截屏检视：

| 截图 | 布局与状态 | 分辨率 | 重点观察面 |
|---|---|---|---|
| [`01_split_summary.png`](screenshots/audit-v0.2/01_split_summary.png) | `Split` 左右分栏 + 激活预设 `prod-api` + 统计空态 | `2400×1520` | 顶栏胶囊与分隔线、过滤状态、`当前列表统计` 卡片、失败请求状态、`底栏` |
| [`02_split_detail_graphql.png`](screenshots/audit-v0.2/02_split_detail_graphql.png) | `Split` 左右分栏 + 选中 GraphQL 请求详情 | `2400×1520` | 详情摘要徽章、Timing 分段条、JSON 正文卡片与搜索/模式切换、`响应头` |
| [`03_wide_three_1200px.png`](screenshots/audit-v0.2/03_wide_three_1200px.png) | `WideThree` 三栏布局 + 无预设仅含 2 条规则 | `2400×1520` | 左栏 `facet_sidebar`、应用/设备图标、根域名树、右栏详情挤压与裁切、顶栏规则预览 |
| [`04_top_bottom_detail.png`](screenshots/audit-v0.2/04_top_bottom_detail.png) | `TopBottom` 上下分栏 + 选中带查询参数与复用连接的请求 | `2400×1520` | 全宽自适应列表列、复用连接标记、上下分栏中详情摘要与左右卡片高度配比 |

---

## 二、P0 高优先级缺陷与布局问题（实机复现）

### P0-1 明文 HTTP 快速建连 / 转发失败漏标 `ERR`，流量永久卡在「进行中」

- **实机复现**（见 [`01_split_summary.png`](screenshots/audit-v0.2/01_split_summary.png) 与 [`03_wide_three_1200px.png`](screenshots/audit-v0.2/03_wide_three_1200px.png) 第 9 行）：
  - 请求 `GET fail.shop.io/health` 通过规则解析到未监听端口 `127.0.0.1:18999`，底层 TCP 建连立即返回 `ECONNREFUSED`。
  - 但界面列表中该行长期停留在状态 `—`、耗时 `—`，右侧「当前列表统计」显示 **`ERR 0`、`进行中 1`**，既无法进入 `失败` / `status:err` 筛选结果，也不显示错误原因。
- **代码根因**：[`app/src/proxy.rs#L3596-L3601`](file:///Users/tree/github/zniff/app/src/proxy.rs#L3596-L3601)（`plain_http` 建连路径）：
  ```rust
  None => timeout(IO_TIMEOUT, dial_timed(&host, port, &hub, false))
      .await
      .context("连接源站超时")
      .inspect_err(|err| {
          hub.set_error(id, explain_flow_error(err));
      })??,
  ```
  `timeout(...)` 返回的是嵌套 `Result<Result<(TcpStream, ConnInfo)>, Elapsed>`。`.context("连接源站超时").inspect_err(...)` **仅拦截外层超时 `Elapsed` 错误**；当 `dial_timed` 在超时前快速返回内部错误（如连接被拒绝、DNS 解析失败）时，外层为 `Ok(Err(e))`，第一个 `?` 直接解开外层、第二个 `?` 带着内部 `Err` 提前退出 `plain_http` 并上抛到连接协程，**完全绕过了 `hub.set_error(id, ...)`**（同函数 [`L3663-L3665`](file:///Users/tree/github/zniff/app/src/proxy.rs#L3663-L3665) 重连分支及 [`L3697`](file:///Users/tree/github/zniff/app/src/proxy.rs#L3697) 读响应头等路径也存在同样直接 `?` 返回却未 `hub.set_error` 的风险，对比 `replay_flow` [`L5715-L5717`](file:///Users/tree/github/zniff/app/src/proxy.rs#L5715-L5717) 在外层统一捕获写错，语义一致）。
- **优化建议**：
  1. 将 `plain_http` 中 `hub.open(...)` 之后到响应完成前的失败路径统一收口（或拆出 `plain_http_inner` 由外层统一 `if let Err(err) = res { hub.set_error(id, explain_flow_error(&err)); }`），确保连接拒绝、DNS 失败、对端关闭和超时都能稳定记录 `ERR` 并标记 `finished`；同时视协议状态酌情向客户端回写 `502 Bad Gateway` 说明或显式关闭。
  2. 同步补充单元测试与端到端回归用例覆盖明文 HTTP 端口拒绝（`ConnectionRefused`）场景。

---

### P0-2 三栏模式（`WideThree`）未计入侧边栏宽度预算，默认窗口下详情栏被严重挤压与裁切

- **实机复现**（见 [`03_wide_three_1200px.png`](screenshots/audit-v0.2/03_wide_three_1200px.png)）：
  - 在默认窗口宽度 `1200 px` 下，左侧 `facet_sidebar` 占固定 `246 px`（[`facet_sidebar.rs#L109`](file:///Users/tree/github/zniff/app/src/ui/facet_sidebar.rs#L109)），中间列表默认 `list_width = 660 px`（[`config.rs#L71`](file:///Users/tree/github/zniff/app/src/config.rs#L71)）外加分割条 `6 px`，留给右栏 `detail_pane` 的宽度仅剩 **`~288 px`**。
  - 扣除详情区左右边距 `pl_3p5()`（14px）+ `pr_5()`（20px）后，卡片内容宽度不足 `254 px`，实机截图出现多处明显视觉问题：
    1. **「响应 body」卡片标题栏右侧控件被硬裁切**（「`美化 | 树形 | 原文`」中 `原文` 右半边在卡片边缘被截断，原因在于 `body_section` 标题栏内「`响应 body`」带 `.min_w(px(96.0))`、`搜索框` 带固定 `120px` 且模式切换条 `.flex_shrink_0()`，超出 `254px` 后受 `.overflow_hidden()` 裁切）；
    2. **详情摘要元数据被截断**：`12:02:52.555 · TTFB 3 ms · 耗时...` 截断无法看清总耗时，`resolve gql:GetDashboa...` 折叠换行，与 `更多 v` 挤成两行；
    3. 如果用户向右拖拽分割条，`clamp_list_px_for_window(width, window_w)`（[`profile_store.rs#L726-L736`](file:///Users/tree/github/zniff/app/src/ui/profile_store.rs#L726-L736)）仍按两栏公式 `window_w - 340.0` 计算上限（`860 px`），未扣除 `246 px` 侧边栏，**可把右栏压缩到仅剩 `~88 px`**。
- **代码根因**：
  - [`app/src/ui/profile_store.rs#L726-L736`](file:///Users/tree/github/zniff/app/src/ui/profile_store.rs#L726-L736) 的 `clamp_list_px_for_window` 及 [`app/src/ui/flow_table.rs#L218`](file:///Users/tree/github/zniff/app/src/ui/flow_table.rs#L218) 未感知 `layout_mode`（是否开启了 `WideThree` 的 `246 px` 侧边栏）。
  - [`app/src/ui/detail_body.rs#L80-L172`](file:///Users/tree/github/zniff/app/src/ui/detail_body.rs#L80-L172) 與 `body_search_box` 在极窄卡片宽度下未自适应缩窄搜索框宽度，导致 `.flex_row().flex_wrap()` 单行内部控件组合仍溢出。
- **优化建议**：
  1. `clamp_list_px_for_window`（或在 `flow_list` / `column_splitter` 传入 `layout_mode`）在 `WideThree` 模式下从可用窗口宽度中扣除侧边栏宽度（`246 px`），保底右栏详情至少保留 `380–420 px`（即默认 `1200 px` 窗口下三栏列表自动收敛到 `~520–560 px`，且不再把 `Split` 模式的 `list_width = 660` 硬套并污染到三栏，可独立记忆或按模式缩放）；
  2. 为 `body_search_box` 增加窄容器自适应宽度（`pane_width < 360.0` 时搜索框由 `120 px` 收窄至 `76–88 px` 或允许 `.flex_shrink_1()`），消除 `原文` 按钮被裁切问题；
  3. 仅保留 `WideThree` 侧栏默认开启阈值提示或允许在 `window_w < 1280` 时一键折叠侧边栏。

---

### P0-3 「当前列表统计」卡片中「总流量」在默认 `1200×760` 窗口即发生截断

- **实机复现**（见 [`01_split_summary.png`](screenshots/audit-v0.2/01_split_summary.png)）：
  - 在标准默认 `1200×760` `Split` 布局下，「当前列表统计」中「总流量」格子显示为 `1.1 KB (↑215 B...`，下行流量 `↓877 B` 完全被省略号截断，而左侧「总请求（`8`）」与右侧「平均耗时（`161 ms`）」各自留有大量空白。
- **代码根因**：[`app/src/ui/detail_summary.rs#L353-L384, L496-L525`](file:///Users/tree/github/zniff/app/src/ui/detail_summary.rs#L353-L384)：
  - 三个 `summary_stat_tile` 使用均等 `.flex_1().min_w(px(128.0))`，且值文本采用 `.number_of_lines(1)` + `.text_overflow(EllipsizeMode::Tail)`。
  - 「总请求」「平均耗时」内容仅 1–6 字符，而「总流量」格式为 `"{total} (↑{req} · ↓{res})"`（约 20+ 字符），在 `534 px` 右栏中均分三格后单格仅约 `148 px`，必然触发尾部截断。
- **优化建议**：
  1. 将 `总流量` 格子权重设为略高于数字格（如 `flex_grow(1.8)`），或把总大小作主数字行（`1.1 KB`）、将 `(↑215 B · ↓877 B)` 作为下方 `11 px` 次级副标题展示，既彻底消灭截断又增强 KPI 视觉层级。

---

### P0-4 应用与设备来源识别准确性与覆盖面有待补强

- **实机复现**（见 [`03_wide_three_1200px.png`](screenshots/audit-v0.2/03_wide_three_1200px.png) 左侧栏）：
  - 10 条测试流量全部来自本机 `127.0.0.1`，其中混有 `curl`、Desktop `CFNetwork`、`iPhone` 与 `Android Dalvik` 四类 `User-Agent`，但左侧「应用 / 来源」却只聚合出 **一条 `📱 127.0.0.1  10`（移动端手机图标）**。
- **代码根因逐项排查**：
  1. **短生命周期 CLI/脚本进程漏识别（回退为裸 `127.0.0.1`）**：[`app/src/proxy.rs#L2070-L2104`](file:///Users/tree/github/zniff/app/src/proxy.rs#L2070-L2104) 在接受 TCP 连接后通过 `tokio::task::spawn_blocking` 异步执行全量 `lsof` 扫描（带 `1500 ms` 缓存）。对于 `curl` 或单次脚本请求，客户端往往在收到响应前后端已进入或完成关闭、`lsof -sTCP:ESTABLISHED` 扫描完成时连接源端口已不在 `ESTABLISHED` 状态，从而回退为裸 `"127.0.0.1"`。
  2. **同源混杂桌面与移动 UA 被强行升级为手机图标**：[`app/src/session.rs#L644-L646`](file:///Users/tree/github/zniff/app/src/session.rs#L644-L646) 规定只要同一来源（哪怕是 `127.0.0.1` 回环地址）出现过一条命中移动 UA 的流量，就将该来源整个条目从 `Desktop` 改为 `Mobile` 手机图标。
  3. **`CFNetwork` 误判 macOS 原生应用为移动端**：[`app/src/session.rs#L689`](file:///Users/tree/github/zniff/app/src/session.rs#L689) `is_mobile_user_agent` 无条件包含 `"cfnetwork"`。而 macOS 上的原生客户端、Xcode、`NSURLSession` 均发送 `AppName/1.0 CFNetwork/... Darwin/...`，当在局域网测试另一台 Mac 或本地回环未抓到进程名时，会被错误展示为手机图标。
  4. **macOS `lsof` 长名称截断与非 ASCII 应用名未转义**：[`app/src/proxy.rs#L169`](file:///Users/tree/github/zniff/app/src/proxy.rs#L169) 调用 `lsof -nP -iTCP -sTCP:ESTABLISHED -Fpcn` 未传 `+c 0`（默认进程名截断 9 字节），且 [`clean_process_name`](file:///Users/tree/github/zniff/app/src/proxy.rs#L365-L368) 仅反转义了 `\x20`，中文或特殊字符 macOS 应用名（如 `微信`、`企业微信`、`飞书`）会变成 `\xe5\xbe\xae...` 原始转义串，进而导致 `app_icon.rs` 查找 `/Applications/<name>.app` 失败。
  5. **子进程未在分面统计中归并到宿主应用**：[`app/src/ui/app_icon.rs#L18-L42`](file:///Users/tree/github/zniff/app/src/ui/app_icon.rs#L18-L42) 在渲染图标时剥离了 `" Helper (Renderer)"` / `" Web Content"` 等后缀，但 [`app/src/session.rs#L634-L651`](file:///Users/tree/github/zniff/app/src/session.rs#L634-L651) 按原始进程名聚合成多条（如 `Google Chrome`、`Google Chrome Helper`、`Google Chrome Helper (Renderer)` 分裂成 3 个分面条目，且用户点选其中一个会漏掉浏览器其余请求的流量；同时 `app:` 过滤若匹配基础前缀或归并分面条目体验会更符合直觉）。
- **优化建议**：
  1. `is_mobile_user_agent` 对 `cfnetwork` 加限定条件（排除同时含 `darwin/` / `macos` / `mac os x` 且不含 `iphone`/`ipad`/`ios` 的 UA）；本机回环未识别到进程名时默认保留桌面/回环语义，避免将回环整体判成移动设备；
  2. macOS `scan_local_ports` 增加 `+c 0`，并用 UTF-8 字节反转义替代仅 `.replace("\\x20", " ")`，解决长进程名截断及中文应用名乱码与图标丢失；
  3. 优化 `lsof` 参数（仅扫描回环/相关端口或在 `127.0.0.1` 建连初期尽早查询缓存），并研究支持通过 `sysctl`/libproc 按源端口查询 PID 与可运行可执行文件路径，减少对外起子进程 `lsof` 的竞态延迟；
  4. 在 `SessionFacets` 或过滤语义中考虑将 `Helper` / `Networking` 子进程归并展示（或提供一级宿主应用分组 + 兼容 `app:` 前缀匹配）。

---

### P0-5 顶栏「过滤胶囊」与「条件摘要」的状态语义歧义

- **实机复现与源码核验**：见 [`03_wide_three_1200px.png`](screenshots/audit-v0.2/03_wide_three_1200px.png) 及 [`app/src/ui/toolbar.rs#L173-L355`](file:///Users/tree/github/zniff/app/src/ui/toolbar.rs#L173-L355)：
  1. **有规则无过滤词的预设，切换状态失灵**：当加载一个只配置了 `rules` / `bypass` / `upstream` 而 `filter = ""` 的预设时，`active_name()` 非空，顶栏显示 `[ ✓ 预设名 ]`；点击它会将 `filter_enabled` 设为 `false`，胶囊视觉变灰且勾号消失（Tooltip 提示“过滤已暂停，点击启用”），但实际上 `rules_on`、`bypass`、`upstream` 全部**仍在生效**，右侧预览甚至不显示 `(已暂停)`，造成「开关关了规则却没停」的强烈误解。
  2. **暂停主过滤时误将激活的二次过滤标为 `(已暂停)`**：在 [`toolbar.rs#L183-L192, L310-L318`](file:///Users/tree/github/zniff/app/src/ui/toolbar.rs#L183-L192) 中，`condition_info()` 传入的是合成后的 `effective_filter`。若用户设了主过滤 `host:*.shop.io`，随后点胶囊暂停主过滤（`filter_enabled = false`），同时在左栏二次搜索栏输入或通过分面胶囊点击了 `status:500`，此时列表**正被 `status:500` 主动筛选**，但顶栏摘要区会在其前面加上 `(已暂停) status:500`——把**正在生效的二次筛选**标成了 `(已暂停)`！
  3. **多条规则前缀 `(2)` 与右侧角标重复且语义模糊**：在 [`03_wide_three_1200px.png`](screenshots/audit-v0.2/03_wide_three_1200px.png) 中，没有预设且未设置主过滤时，顶栏显示 `(2) resolve api.shop.io|auth.shop.io|...`，右侧按钮又显示 `● 过滤 2`。中间的 `(2)` 缺少主语（易被误解成某种计数/序号），且长规则直接展示原始文本容易在中部被 `...` 截断。
- **优化建议**：
  1. 仅当配置中**确实存在主列表 `filter`**（`!profiles.filter.get().trim().is_empty()`）时才将该胶囊作为「过滤启停 Toggle」，否则若仅包含规则/上游/直通，保留为只读指示胶囊（或点击直接打开「过滤」面板 / 明确标明“含 N 条规则”而非“过滤已暂停”）；
  2. `toolbar_filter_conditions` 明确区分 `primary_filter`（随 `filter_enabled` 切换并标注 `(主过滤已暂停)`）与 `list_filter`（始终生效的会话筛选，可单独以轻量tag如 `+ status:500` 展示），避免将活跃的二次过滤误标为已暂停；
  3. 多条规则在顶栏预览从 `(2) <第一条规则全文>` 优化为更清晰的摘要（例如 `规则 (2): resolve …` 或 `2 条规则已启用 · 点击配置`），避免与右侧数字角标冗余。

---

## 三、P1 核心交互与效率优化建议

### P1-1 三栏侧边栏与分面抽屉（`facet_sidebar` / `facet_drawer`）交互效率提升

对照 [`03_wide_three_1200px.png`](screenshots/audit-v0.2/03_wide_three_1200px.png) 与 [`app/src/ui/facet_sidebar.rs`](file:///Users/tree/github/zniff/app/src/ui/facet_sidebar.rs)：

1. **根域名分组行缺少「一键筛选 / 屏蔽整组」能力**：当前多子域聚合为根域名行（如 `shop.io  8`）时，点击整行只能展开/收起子域（[`facet_sidebar.rs#L409-L416`](file:///Users/tree/github/zniff/app/src/ui/facet_sidebar.rs#L409-L416)）。在微服务域名众多时，用户非常高频需要一键切入整组泛域名筛选（`host:*.shop.io|shop.io`）或一键屏蔽整组第三方上报域（如 `mute:*.qq.com`）。建议保留左侧小箭头区域负责折叠，点击根域名文字一键切换组过滤、右侧保留 `×` 屏蔽整组。
2. **已统计的 `error_count` 未在列表行展现**：`session::FacetEntry` 已实时维护了每个域名和应用的 `error_count`（[`session.rs#L603, L630, L648`](file:///Users/tree/github/zniff/app/src/session.rs#L603)），但 `facet_sidebar` / `facet_drawer` 目前只渲染了总请求数 `count`。对于联调定位接口报错，在有错误的分面上展示紧凑的红色 `error_count` 角标（如 `4/10` 或小红点数值）能显著缩短定位路径。
3. **长域名 / 长应用名省略后缺少悬停全文提示**：[`facet_sidebar.rs#L358, L542`](file:///Users/tree/github/zniff/app/src/ui/facet_sidebar.rs#L358) 对超过 `246 px` 的域名与进程名做了 `Middle`/`Tail` 截断，但未附带 `.tooltip(...)`，遇到多个仅有微弱前缀或中缀差异的微服务端点域名时无法确认全名。
4. **侧边栏固定 `246 px` 不可调宽也不可折叠，且未保存分段折叠状态**：可考虑增加可拖拽分割条（记忆 `sidebar_width`）或顶栏左侧一键折叠开关；并将「应用 / 来源」与「域名」列表使用本地状态记忆展开状态。
5. **非本地回环设备 IP 缺少轻量设备提示**：来自局域网手机/平板代理的流量目前在「应用 / 来源」仅展示纯数字 IP（如 `10.49.83.239`）。若在 `SessionFacets` 中为每个远端 IP 附带从最近/首条请求提取的系统提示标签（如 `10.49.83.239 · iOS`、`10.49.83.240 · Android`、`10.49.83.12 · macOS`），无需记住各测试机 IP 即可秒辨设备来源。

---

### P1-2 上下分栏（`TopBottom`）纵向空间配比与「过滤」弹出层布局适配

1. **`TopBottom` 下摘要区域挤压请求/响应卡片高度**（见 [`04_top_bottom_detail.png`](screenshots/audit-v0.2/04_top_bottom_detail.png)）：
   - `1200×760` 窗口下，底栏详情高度约 `370 px`，其中顶部的 `summary` 摘要卡片横跨全宽占去约 `145 px`（近 40% 高度），使得下方的「响应 body」「查询参数」「请求头」卡片只能展示约 6–8 行便开始滚动。
   - 建议在 `LayoutMode::TopBottom` 时允许摘要采用更紧凑的排布（例如将「状态+URL+指标行」压缩边距，或将耗时分段条折叠进右侧/支持双列头部），把更多纵向空间留给左右对比的正文与头部。
2. **`adaptive_filter_panel_width` 未适配 `TopBottom` 与 `WideThree`**：
   - [`app/src/ui/profile_store.rs#L760-L771`](file:///Users/tree/github/zniff/app/src/ui/profile_store.rs#L760-L771) 中 `adaptive_filter_panel_width(window_w, list_w)` 仍按两栏模式公式 `window_w - list_px * 0.7 - 16.0` 计算弹出层宽度。在 `TopBottom`（没有左右分割的概念）或 `WideThree`（左侧还有 `246 px` 侧栏）下，弹窗宽度仍受历史 `list_width` 影响。建议按 `layout_mode` 选择合理的自适应预算（例如 `TopBottom` / `WideThree` 采用稳定比例 `(window_w * 0.78).clamp(740.0, 1280.0)`）。

---

### P1-3 规则文本编辑可维护性与「过滤」按钮视觉冗余优化

1. **已有规则行缺乏单行开关与回填编辑**：目前底部 15 项分段表单（[`quick_rules.rs`](file:///Users/tree/github/zniff/app/src/ui/quick_rules.rs)）仅支持「向规则框末尾追加一行」；当一条 `replace`、`json` 或 `map-local` 规则需要微调或临时注释测试时，只能手动修改多行文本或手打 `#`。可在规则框旁（或校验预览条）提供「注释/启用当前行」或从下拉清单点击回填表单重新生成替换该行，大幅降低语法记忆负担。
2. **「过滤」按钮（`filter_panel.rs:32-135`）在有规则/预设时样式过载**：见 [`01_split_summary.png`](screenshots/audit-v0.2/01_split_summary.png) 顶栏，按钮内部同时出现 `[ 蓝点 + 滑块图标 + 过滤 + 蓝底数字角标 ]` 四个视觉元素，而 `●` 蓝点与右侧 `1` 数字角标和蓝色底纹表达了重叠的状态信息。当规则计数角标已展示或面板未暂停规则时，可省略左侧状态圆点，仅在规则/断点被关掉（警告态）时高亮警告点，视觉更干净统一。

---

### P1-4 详情面板、请求构造与对比视图的日常顺手度

1. **「构造请求」草稿与最近请求历史**（[`compose_box.rs#L758-L829`](file:///Users/tree/github/zniff/app/src/ui/compose_box.rs#L758-L829)）：每次取消编辑（`reset_compose`）或重启窗口都会清空手动编写的 Method/URL/Headers/Body；未发送或调试一半误按 `Esc` 后无法恢复。建议在会话内保留上次未发送的「新建请求」草稿（或保留最近 5 条构造历史供下拉填充）。
2. **多页正文（`> 256 KB`）搜索提示与跨页跳转**：当前卡片搜索仅作用于当前 `256 KB` 分页窗口（并在标题旁标注「当前页命中」），但未提示前后页是否也有命中或提供「下一匹配页」跳转；当接口返回接近 `1 MB` JSON 且关键字落在后半段时，需要手动逐页点击「往后」查找。
3. **行级对比支持筛选聚焦**：当前对比视图将 URL、请求头、请求 Body、响应头、响应 Body 拼合后渲染（每侧 200 行上限）。当请求头或 Cookie 很长时，核心关注点（响应 Body 差异）容易被头部差异挤到下方，增加轻量维度筛选（`全部` / `仅 Body` / `仅 Headers`）可立刻聚焦业务差异。

---

## 四、P2 多端一致性、国际化与 AI MCP 增强

1. **中继来源标签未跟随界面语言本地化**：[`app/src/session.rs#L674-L676`](file:///Users/tree/github/zniff/app/src/session.rs#L674-L676) 中 `relay_label` 直接格式化为中文 `"中继 {short}"` 并写入 `flow.client_ip`，在界面切换为英文（`Lang::En`）时列表「来源 IP」列与分面抽屉仍显示中文「中继」，应改为在展示层格式化或在列表/分面渲染中本地化前缀。
2. **内置 MCP Server（[`app/src/mcp.rs`](file:///Users/tree/github/zniff/app/src/mcp.rs)）可补齐会话清理与断点控制 Tool**：
   - 核实源码确认 `mcp.rs` 目前已提供 **7 个 Tool**：`list_flows`、`get_flow_detail`、`get_session_summary`（已复用 `FilteredStats` 与 `SessionFacets`）、`annotate_flow`、`replay_flow`、`get_rules`、`set_rules`，已非常完整。
   - 唯一明显的自动化联调断点是：AI 代理在重放验证或下发 `break` / `break-res` 规则后，尚无 MCP Tool 可查询或放行/丢弃当前 `paused_queue` 断点（如 `list_breaks` / `resolve_break`），也没有清理旧测试流量的 `clear_flows`（可支持 `keep_pinned: bool`）。补齐这两类轻量 Tool 后，AI 联调闭环将彻底完整。
3. **Web 操作台（[`app/src/web.rs`](file:///Users/tree/github/zniff/app/src/web.rs)）在缺失 `app/.nsh/web` 时的应急页与客户端一致性提示**：目前桌面端新增的三栏布局、分面图标（macOS `.app` 提取）在纯 Wasm 环境下需依赖静态 UA 图标回退，可在 UI 文档和提示中注明跨端表现一致性边界。

---

## 五、P3 规范文档与当前实现同步清单（Doc Drift）

审计过程中核对 `AGENTS.md` 与 `docs/known-issues.md` 时，发现近期若干高价值演化已在代码落地但规范文档尚未同步更新，建议后续统一刷新：

| 主题 | 旧文档描述（`AGENTS.md` / `docs/known-issues.md`） | 当前代码真实实现 | 代码位置 |
|---|---|---|---|
| 主工作区布局 | “界面只有两栏” | 已支持 `Split`（左右分栏）、`TopBottom`（上下分栏，右区响应与请求并列）、`WideThree`（左侧 `246px` 应用/域名树 + 列表 + 详情）三种模式 | [`config.rs#L210-L224`](file:///Users/tree/github/zniff/app/src/config.rs#L210-L224), [`app_shell.rs#L978-L1044`](file:///Users/tree/github/zniff/app/src/ui/app_shell.rs#L978-L1044) |
| 列表 Path 列展示模式 | 仅描述 `show_full_url` 二态（「仅路径 / 完整 URL」） | 已升级为四态 `PathDisplay`：`Auto`（智能按主机列/过滤词切换仅路径或 Host+Path）、`PathOnly`、`HostAndPath`、`FullUrl` | [`config.rs#L157-L188`](file:///Users/tree/github/zniff/app/src/config.rs#L157-L188), [`list_columns.rs#L488-L524`](file:///Users/tree/github/zniff/app/src/ui/list_columns.rs#L488-L524) |
| 顶栏激活配置胶囊 | `[<配置名> ×]`（点击 `×` 取消选择配置） | 已演进为单胶囊开关 `[ ✓ <配置名> ]`（点击切换主过滤启用/暂停；无过滤词或未选配置时表现需按 P0-5 收敛） | [`toolbar.rs#L207-L311`](file:///Users/tree/github/zniff/app/src/ui/toolbar.rs#L207-L311) |
| 代理认证配置入口 | “过滤面板右栏上游下面包含代理认证、中转地址、中转密钥与接收中继” | 「代理认证」已独立迁至底栏右侧「认证」弹出层；「中转设置」已拆为「更多 → 中继设置」独立弹窗 | [`footer_qr.rs#L160, L187`](file:///Users/tree/github/zniff/app/src/ui/footer_qr.rs#L160), [`machine_settings.rs#L5`](file:///Users/tree/github/zniff/app/src/ui/machine_settings.rs#L5) |
| 分面导航位置 | “搜索框右侧可折叠按钮展开上方抽屉” | 两栏/上下分栏改为列表顶部常驻快选条 + 浮动覆盖层抽屉；三栏改为左侧常驻树；已接入应用图标与移动/PC 识别 | [`facet_drawer.rs#L388-L586`](file:///Users/tree/github/zniff/app/src/ui/facet_drawer.rs#L388-L586), [`app_icon.rs#L196`](file:///Users/tree/github/zniff/app/src/ui/app_icon.rs#L196) |
| MCP Tool 数量 | `docs/known-issues.md` 记载为 5 个 Tool | 实际已扩展为 7 个 Tool（含 `get_session_summary` 与 `annotate_flow`） | [`mcp.rs#L48-L179`](file:///Users/tree/github/zniff/app/src/mcp.rs#L48-L179) |

---

## 六、建议迭代优先级路线图

| 优先级 | 优化项 | 预期收益 | 预估改动量 | 涉及主要模块 |
|---|---|---|---|---|
| **1 (P0)** | 修复 `plain_http` 建连/转发快速错误漏调 `hub.set_error` 导致卡在「进行中」 | 消除明文 HTTP 失败请求永远显示 `—` 与统计漏计 `ERR` 的准确性缺陷 | 小（1–2h） | [`proxy.rs`](file:///Users/tree/github/zniff/app/src/proxy.rs) |
| **2 (P0)** | `WideThree` 分栏扣除 `246px` 侧栏宽度上限 + 窄详情页正文工具栏自适应 | 彻底解决三栏默认窗口右侧 `原文` 按钮裁切、摘要截断与拖拽压死右栏 | 小–中（2–3h） | [`profile_store.rs`](file:///Users/tree/github/zniff/app/src/ui/profile_store.rs), [`detail_body.rs`](file:///Users/tree/github/zniff/app/src/ui/detail_body.rs) |
| **3 (P0)** | 修复「总流量」KPI 卡片截断 + 修正顶栏 `[✓ 配置]` 与 `(已暂停)` 语义歧义 | 消除默认窗口首页视觉截断与“暂停按钮不生效/误标二次筛选”困惑 | 小（1–2h） | [`detail_summary.rs`](file:///Users/tree/github/zniff/app/src/ui/detail_summary.rs), [`toolbar.rs`](file:///Users/tree/github/zniff/app/src/ui/toolbar.rs) |
| **4 (P0)** | 完善应用/设备识别（修 `CFNetwork` 误判、macOS `lsof +c 0`/UTF-8 反转义、短连接竞态优化） | 消除 `127.0.0.1` 误标手机图标、中文 App 乱码与长进程名被截断 | 中（3–4h） | [`session.rs`](file:///Users/tree/github/zniff/app/src/session.rs), [`proxy.rs`](file:///Users/tree/github/zniff/app/src/proxy.rs), [`app_icon.rs`](file:///Users/tree/github/zniff/app/src/ui/app_icon.rs) |
| **5 (P1)** | 分面侧边栏补齐根域名一键筛选/屏蔽、`error_count` 角标、悬停全名与远端设备 OS 提示 | 三栏主导航真正成为大流量下的异常定位与设备分辨利器 | 中（3–4h） | [`facet_sidebar.rs`](file:///Users/tree/github/zniff/app/src/ui/facet_sidebar.rs), [`facet_drawer.rs`](file:///Users/tree/github/zniff/app/src/ui/facet_drawer.rs), [`session.rs`](file:///Users/tree/github/zniff/app/src/session.rs) |
| **6 (P1)** | `TopBottom` 紧凑摘要、`adaptive_filter_panel_width` 按布局适配、「过滤」按钮去重微调 | 提升上下分栏正文可视行数与过滤弹出层宽度一致性 | 小–中（2h） | [`detail_summary.rs`](file:///Users/tree/github/zniff/app/src/ui/detail_summary.rs), [`filter_panel.rs`](file:///Users/tree/github/zniff/app/src/ui/filter_panel.rs) |
| **7 (P2)** | `relay_label` 双语化、MCP 补齐 `clear_flows`/`break` 控制、构造请求历史草稿、刷新规范文档 | 完善 AI 自动化闭环与英文界面/文档一致性 | 中（3–5h） | [`session.rs`](file:///Users/tree/github/zniff/app/src/session.rs), [`mcp.rs`](file:///Users/tree/github/zniff/app/src/mcp.rs), [`compose_box.rs`](file:///Users/tree/github/zniff/app/src/ui/compose_box.rs), `docs/` |

---

## 七、优化落地与实机截图复测（已完成）

上述 P0–P3 全部优化已落地并通过全量检查与实机 Retina（`2400×1520`）复测，主要改动与实机验证对比如下：

1. **P0 核心可靠性与布局修复**：
   - [`app/src/proxy.rs`](file:///Users/tree/github/zniff/app/src/proxy.rs)：将 `plain_http` 拆分为 `plain_http` + `plain_http_inner`，并在 `timeout(IO_TIMEOUT, dial_timed(...))` 处确保连接失败立即调用 `hub.set_error`；实机 [`05_after_split_summary.png`](screenshots/audit-v0.2/05_after_split_summary.png) 中 `fail.shop.io/health` 正确显示红字 `ERR`（`进行中 0`、`ERR 1`），不再永久卡在 `—`。
   - [`app/src/ui/detail_summary.rs`](file:///Users/tree/github/zniff/app/src/ui/detail_summary.rs)：采用按权重分配的 `summary_stat_tile` 修复 `总流量` 卡片截断，实机完整展示 `1.1 KB ↑215 B · ↓879 B`。
   - [`app/src/ui/profile_store.rs`](file:///Users/tree/github/zniff/app/src/ui/profile_store.rs)、[`app/src/ui/facet_drawer.rs`](file:///Users/tree/github/zniff/app/src/ui/facet_drawer.rs)、[`app/src/ui/detail_body.rs`](file:///Users/tree/github/zniff/app/src/ui/detail_body.rs)：`WideThree` 下依据窗口宽度与侧栏宽度实时约束列表最大宽度（在 `1200×760` 下为详情区保留 `≥396 px`），配合自适应宽度的 `body_search_box`；实机 [`06_after_wide_three_1200px.png`](screenshots/audit-v0.2/06_after_wide_three_1200px.png) 右栏完整显示 `搜索…` + `美化 | 树形 | 原文`，无裁切溢出。
   - [`app/src/ui/toolbar.rs`](file:///Users/tree/github/zniff/app/src/ui/toolbar.rs)、[`app/src/ui/filter_panel.rs`](file:///Users/tree/github/zniff/app/src/ui/filter_panel.rs)：仅当主过滤词非空时允许顶栏胶囊切换启用/暂停；主过滤为空但存在规则时展示 `规则×N:` 预览且点击直接展开配置面板，去除 `过滤` 按钮多余蓝点。
   - [`app/src/local_proc.rs`](file:///Users/tree/github/zniff/app/src/local_proc.rs)（从 `proxy.rs` 拆出独立模块）、[`app/src/session.rs`](file:///Users/tree/github/zniff/app/src/session.rs)：采用 `lsof +c 0` + 并发受控单端口探测（不跨外部命令持锁）、完整 `\xHH` UTF-8 反转义、任意 `Helper (...)` 子进程后缀折叠、去除裸 `CFNetwork` 与孤立 `127.0.0.1` 的移动端误判；实机 [`06_after_wide_three_1200px.png`](screenshots/audit-v0.2/06_after_wide_three_1200px.png) 左侧清晰聚合 `curl`（带终端图标）与 `127.0.0.1`（桌面图标）。
2. **P1 效率、空间与辅助编辑增强**：
   - [`app/src/ui/facet_sidebar.rs`](file:///Users/tree/github/zniff/app/src/ui/facet_sidebar.rs) & [`app/src/ui/filter_tokens.rs`](file:///Users/tree/github/zniff/app/src/ui/filter_tokens.rs)：侧栏支持 `176–340 px` 右缘拖拽调宽与 `<` 折叠至 `30 px` 展开条、根域名 `shop.io` 三段式组头整组筛选（`host:*.shop.io|shop.io`）与 `×` 屏蔽、所有分面/根域名行展示红色异常计数角标及全名 Tooltip。
   - [`app/src/ui/detail_summary.rs`](file:///Users/tree/github/zniff/app/src/ui/detail_summary.rs) & [`app/src/ui/filter_panel.rs`](file:///Users/tree/github/zniff/app/src/ui/filter_panel.rs)：实机 [`07_after_top_bottom_detail.png`](screenshots/audit-v0.2/07_after_top_bottom_detail.png) 在 `TopBottom` 下将 URL 与耗时指标合并为单行紧凑摘要，为下方正文与头部卡片留出更高可视区；同时按 `LayoutMode` 适配过滤弹出层宽度。
   - [`app/src/ui/rule_assist.rs`](file:///Users/tree/github/zniff/app/src/ui/rule_assist.rs)、[`app/src/ui/compose_box.rs`](file:///Users/tree/github/zniff/app/src/ui/compose_box.rs)、[`app/src/ui/detail_body.rs`](file:///Users/tree/github/zniff/app/src/ui/detail_body.rs)、[`app/src/present.rs`](file:///Users/tree/github/zniff/app/src/present.rs) & [`app/src/ui/detail_headers_msgs.rs`](file:///Users/tree/github/zniff/app/src/ui/detail_headers_msgs.rs)：新增规则编辑器单行 `#` 注释切换/回填辅助条、「构造请求」未发送草稿保留与最近 8 条请求历史下拉及清空、`>256 KB` 分页正文「当前页 · 下一处」跨页跳转，以及行级对比 `[全部 | 仅正文 | 仅头部]` 维度筛选。
3. **P2–P3 MCP 闭环与文档同步**：
   - [`app/src/session.rs`](file:///Users/tree/github/zniff/app/src/session.rs)：中继来源标签 `format_client_label` 按界面语言切换为 `"中继 …"` / `"Relay …"`。
   - [`app/src/mcp.rs`](file:///Users/tree/github/zniff/app/src/mcp.rs)：新增 `clear_flows` 与 `manage_breaks`（支持 `list`/`continue`/`continue_all`/`drop`），MCP 工具集扩充至 **9 个 Tool**。
   - 已同步更新 [`AGENTS.md`](file:///Users/tree/github/zniff/AGENTS.md) 与 [`docs/known-issues.md`](file:///Users/tree/github/zniff/docs/known-issues.md)。
4. **全套验证结果**：`make fmt`、`make lint`（`clippy --all-targets -- --deny=warnings` 零告警）、`make check-wasm` 与 `make test`（**383 passed, 0 failed**）均一次性通过，且验证过程中未修改系统 `Wi-Fi` 代理设置。

| 优化后复测截图 | 验证重点 |
|---|---|
| [`05_after_split_summary.png`](screenshots/audit-v0.2/05_after_split_summary.png) | `fail.shop.io/health` 正确标记 `ERR`（`进行中 0` / `ERR 1`）、`总流量` 完整显示上下行字节、`过滤 2` 去除多余蓝点 |
| [`06_after_wide_three_1200px.png`](screenshots/audit-v0.2/06_after_wide_three_1200px.png) | `1200×760` 三栏右区完整展示 `美化 \| 树形 \| 原文`、顶栏 `规则×2:` 预览、侧栏折叠按钮 / 红标异常数 / `curl` 应用归属 |
| [`07_after_top_bottom_detail.png`](screenshots/audit-v0.2/07_after_top_bottom_detail.png) | `TopBottom` URL 与指标单行紧凑摘要，响应正文与请求参数/头面板可视高度提升 |
