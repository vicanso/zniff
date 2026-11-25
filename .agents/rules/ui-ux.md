---
trigger: always_on
---

# zniff UI / UX 与动效约束

- **NSH 原生组件优先与设计规范对齐**：
  - **写组件前先确认 NSH 是否有相应组件**：新增或重构任何 UI 控件、布局原语或浮层前，必须先查阅本机 `nsh-base` / `nsh-core` 源码或 `https://nsh.dev/elements/{name}.md`，优先直接复用原生组件（`button`、`tabs`/`tab`、`select`、`sheet`、`resize_handle`、`collapsible`/`accordion`、`checkbox`、`radio_group`/`radio`、`switch`、`link`、`popover`、`menu`、`hover_card`、`tooltip`、`toast_stack`、`data_table`、`tree`、`command`、`editor`、`diff_view`、`text_view`、`progress`、`virtual_list` 等），禁止手写 `view().role(AccessibilityRole::Button / Tab / Checkbox / Radio / Link)` 或重复造拖拽/浮层轮子（`popover()` / `menu()` 的 `.trigger(...)` 等明确要求 `View` 或无障碍宿主绑定者除外）；仅当 `nsh-base` 缺失或存在已核实的业务不兼容（如多 token DSL 过滤框、含自定义 `Esc` 堆栈的 5 个主弹窗）时才允许自建并说明原因。
  - **设计规范尽量参考 NSH**：屏幕布局、信息层级、间距与密度、组件选型、键盘导航（`Tab`/方向键/`Enter`/`Space`/`Esc`）、焦点可见性、无障碍语义（`accessibility_label` / `accessibility_state`）与动效节奏（含 `reduce_motion()`），优先遵循 `nsh-design-guides` 与 NSH 官方设计/编码规范（`https://nsh.dev/docs/design-guides.md`、`coding-guides.md`），再叠加 `zniff` 主题色与桌面/Web 适配约束。
- **NSH 0.7 核心约束**：组件函数只在挂载时运行一次，会变的状态必须用 `text(move || ...)`、`computed(...)`、`when(...)`、`each(...)`、`dynamic(...)`、`style_with(...)`；原语从 `nsh_core::prelude` 引入，基础组件用 `nsh_base`。
- **DataTable 严禁行为**：禁止在格子内放 `tooltip()`、`context_menu()` 或绑定外部长寿命 props 的组件（行复用会引发 `prop read or written after its owner was disposed` panic）；禁止把表格包进 `context_menu`（行高会塌为 0）；不要额外画常驻竖向滚动条，也不要给 `fixed_edge` 上色；行右键/悬停提示通过 `CellHover` 浮层放在表格旁层，右键菜单只修改 `picked` 临时信号，禁止未确认时改写 `selected` 或清除 Compose 草稿。
- **滚动与裁剪防穿透**：弹出层或自适配高度父容器内的 `scroll()`（带 `.w_full()` / `.max_h(...)`）禁止加 `.flex_1()`，否则 `flex-basis: 0` 会将内容压成 `0px`；带内部 `editor()` / `diff_view()` / 嵌套 `scroll()` / `virtual_list()` 的外层滚动内容节点必须包一层 `uncull_nested_scroll(view())`，且每个 `editor()` 与 `diff_view()` 外容器必须使用 `guard_culled_pointer_events(view())`。所有弹出层、抽屉、弹窗必须加随 `window()` 约束的 `max_w` / `max_h` 并兜底内部 `scroll()`。
- **统一动效与 Kit (`app/src/ui/kit/`)**：
  - 时长/缩放统一使用 `tokens.rs` 常量（`MOTION_PRESS_MS`、`MOTION_EXIT_MS`、`MOTION_INSTANT_MS`、`MOTION_FAST_MS`、`MOTION_BASE_MS`、`MOTION_PULSE_MS`、`PRESS_SCALE`、`PRESS_SCALE_ICON`），过渡时长用 `.duration(move || motion_ms(...))` 响应式响应系统 `reduce_motion()`。
  - `popover()` / `menu()` / `hover_card()` 统一成对设置 `.popup_style(ui_popup_style)`、`.popup_starting_style(ui_popup_starting_style)`、`.popup_ending_style(ui_popup_ending_style)`；弹窗用 `ui_modal_backdrop` / `ui_modal_card`；旋转折叠箭头用 `ui_rotating_chevron`；脉动状态点与骨架用 60fps `ui_pulse_animation`；侧边栏折叠调用 `SidebarLayoutRef::animate`。
  - `.scale(...)` / `.opacity(...)` 闭包返回值显式写成 `1.0_f32` / `0.0_f32`；`KeyframeTiming` 为 `#[non_exhaustive]`，先 `default()` 再逐字段赋值。
- **体验、文案与性能底线**：
  - 文案全部用 `tr("中文", "English")`（非 UI 持有 `Lang` 处用 `t(lang, zh, en)`）。全局术语统一用「过滤」（禁用「筛选」），中文标点使用全角括号 `（）` 与 `…`。数字列、倒计时与计数加 `.tabular_nums()`。
  - 破坏性操作（清空未星标、清空 Compose、重置 Allow Hosts、删规则/备注/标记/提醒等）必须带置灰禁用、草稿缓冲或 3.5 秒二次点击确认；Mock 入口统一调用 `begin_inline_mock`；复杂系统/状态变更自动补齐前提且反馈诚实（如 CA 未信任提示 `CaInstalledNeedsTrust`，自定义网络状态不误选中「不限速」）。
  - 高频过滤/解析走 `computed(...)` 记忆化；`CellHover` 走 280ms hover-intent 防抖并支持离开取消；悬停按钮保留固定宽高占位（如 22px）避免布局抖动并用本地轻量 ✓ 确认；阻塞 OS 探测（GPU 等）放后台线程。
- **状态切换与回归测试门禁**：
  - **单一状态转换口 + 保留/复位清单**：凡是支持切换对象/模式且组件会原地复用的界面（切换 `selected` 流、切换规则/表单类型、翻页、取消选中、关闭抽屉/浮层），必须显式划分「跨切换保留状态」（如通用匹配目标 `host`/`path`/`op`、同流刷新时的滚动位置）与「切换必复位状态」（如语义不兼容的动作字段 `from`/`to`、内部 `editor()`/`scroll()` 偏移、受控选中态），收口到单一转换函数（如 `switch_quick_kind`、`on_body_scroll_reset`、`table_selection_for_row`）供所有 UI 入口复用，禁止在多个按钮/菜单里各自直接 `.set(...)`。
  - **带前置脏态的 A → B 无头回归测试**：新增或修改此类交互时，禁止只测“空表单/初始态 + 单次 Happy Path”，必须补充带前置脏态（已填值、已滚动、已选中、已展开）的无头 UI 转移测试，覆盖：（1）`A → B` 切换后局部/不兼容状态已复位且共享上下文保留；（2）`A → A'` 同对象增量刷新不误重置滚动或输入；（3）`Esc` / 点外部区域或外部输入框 / 关闭按钮的退出链路与视觉状态一致；（4）至少穿透一处真实 UI 控件交互（菜单/下拉/按钮点击）而非仅测纯函数。
- **主题、字体、窄窗与移动端 Web**：
  - 配色用 `AdaptiveColor::rgb` 做深浅分层；默认等宽字号跟随 `scaled_px(...)` 与 `ui_zoom`（含 `Text`、`DataTable` 与 `Editor`）；窄窗口 `Split` 宽 `< SIDEBAR_AUTO_COLLAPSE_PX`（`880.0`）时由壳层自动收起侧栏并切出顶栏抽屉（保持独立小组件单元测试隔离）。
  - `/zniff`（`local_routes.rs`、`ios_guide.rs`、`android_guide.rs`）与 `/s/{uuid}`（`share.rs`）须支持浅深色随系统、`viewport-fit=cover` + `safe-area-inset`、`100dvh`、触控热区、纯 HTTP 局域网 `execCommand` 复制降级与 `prefers-reduced-motion`。
- **同步频率**：工作线程只写 `Mutex`，界面约每 16 ms 同步到 `Signal`；禁止把整份 `Session` 塞进单个 `Signal`。
