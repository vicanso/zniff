# zniff NSH 脚本性能、沙箱与包体红线

本规则约束 `zniff` 中所有基于 TypeScript / TSX 与沙箱化 QuickJS（`quickjs-jit`）的自定义扩展能力（工作区布局脚本与后续自定义抓包载荷解码/展示脚本）。核心原则：**脚本能力绝对不能拖慢抓包主链路、列表渲染、冷启动或日常内存与体积。**

---

## 1. 宿主模型与二进制体积红线

- **纯 Rust 根组件不变**：桌面与 Web 控制台统一维持 `app/nsh.json` 的 `"root": "App"` 纯 Rust 根组件架构；脚本仅用于声明式输出工作区 `LayoutSpec` 或单条流的详情展示 IR，由 Rust 原生 NSH 组件负责渲染与交互。
- **严禁启用重型 JS 运行时或编译器**：
  - 禁止开启 `nsh-engine/js`（避免拉入 `nsh-js`、`nsh-package`、`oxc_sourcemap` 等额外 1.5–2.5 MiB 运行时）。
  - 禁止引入 `swc`、`oxc`、`deno_core`、V8 或完整 TypeScript 编译器；TSX/TS 剥离与 JSX 转换继续由零依赖轻量编译器 [`layout_tsx.rs`](../../app/src/layout_tsx.rs) / [`layout_jsx.rs`](../../app/src/layout_jsx.rs) 承担。
- **体积硬指标**：
  - `rquickjs`（`quickjs-jit`）必须严格限定在 `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` 下，**桌面 Release 总增量 `< 1 MiB`**（实测 `~700 KiB` / `~3.5%`）。
  - **`wasm32` 目标 JS 引擎增量为 `0 B`**：浏览器端永不打包或初始化 QuickJS，相关校验/转换请求经 `/api/*` 交由服务端异步执行。

---

## 2. 抓包与大列表热路径绝对禁区

- **代理转发链路禁区**：脚本**严禁**进入 `app/src/proxy/*`（HTTP/1.1、HTTP/2、TLS MITM、WebSocket/SSE）转发路径与连接管理；若需要改包/断点/拦截，继续使用纯 Rust 规则引擎（`rules.rs` / `rule_vars.rs`）与本机回环 HTTP `hook`（`webhook.rs`）。
- **入库与列表投影禁区**：脚本**严禁**在 `Session` 写入、`matching_with_previous_start` 全量过滤扫描、`FilteredStats` / `SessionFacets` 聚合或 `row_projection::project_rows`（10k/100k 流列表）中执行。
- **仅允许两个旁路场景**：
  1. **工作区布局**：低频切换或窗口尺寸越过断点时产出 `LayoutSpec`。
  2. **自定义包转换展示**：仅在用户点选某条流（`selected` 或对比 `baseline`）检视详情时，于后台线程按需异步解码。

---

## 3. 线程模型、冷启动与闲置内存回收

- **单一懒加载后台工作池（[`script_worker.rs`](../../app/src/script_worker.rs)）**：
  - 全部沙箱脚本执行统一收口到单线程后台工作器 `zniff-script-worker`（有界队列 `SCRIPT_QUEUE_CAP = 16` + 最新请求合并）。
  - **UI 主线程与 `--web` Tokio HTTP 工作线程严禁直接创建或阻塞运行 `rquickjs::Runtime`**；`web.rs` 必须通过 `eval_layout_async` 异步 `await`。
- **零成本未启用路径**：
  - 未配置自定义脚本（如 `active_layout == ""`）时：**0 后台线程、0 QuickJS VM、启动与托盘重开 0 目录扫描、0 `.d.ts` 写入**；仅当用户打开工具栏布局菜单或设置面板时按需懒加载列表。
  - 已启用自定义布局时：在 `<data_dir>/layouts/.active-cache.json` 与 `AppRuntime.cached_active_layout` 缓存匹配桶位的 `LayoutSpec`，**冷启动与托盘重开首帧 `0 VM` 同步挂载不闪烁**。
- **热复用与自动回收**：
  - `zniff-script-worker` 内部热复用已加载 `SANDBOX_PRELUDE` 的 `WarmLayoutVm` 与有限转译缓存（禁止跨调用重建 VM）。
  - **闲置 `60s`（`SCRIPT_WORKER_IDLE_TTL`）或窗口最小化到托盘 / 进程退出时**，必须调用 `shutdown_script_worker()` 立即销毁 VM 并退出线程归还内存；清理会话（`clear_captured_session`）必须调用 `clear_script_caches()`。

---

## 4. 分级沙箱配额、字节缓存与熔断

| 脚本类型 | 内存上限 | 栈上限 | 超时上限 | I/O 负载与缓存边界 |
| :--- | :--- | :--- | :--- | :--- |
| **布局脚本** | `8 MiB`（`LAYOUT_MEM_LIMIT_BYTES`） | `256 KiB` | `25 ms`（`LAYOUT_TIMEOUT_MS`） | 源码 `<= 128 KiB`；Worker 堆超 `4 MiB` 或超时即重建 VM |
| **包转换脚本** | `16 MiB`（`PACKET_MEM_LIMIT_BYTES`） | `256 KiB` | `80 ms`（`PACKET_TIMEOUT_MS`） | **原生 `Uint8Array` 直传**（禁 Base64 膨胀）；输入 `<= 1 MiB`、输出 IR `<= 512 KiB` |

- **字节上限 LRU 缓存（`ByteBoundedLru`）**：包转换结果缓存必须同时受**总字节 `<= 8 MiB`（`PACKET_CACHE_MAX_BYTES`）与条目 `<= 64`（`PACKET_CACHE_MAX_ENTRIES`）**双约束；清屏或进入托盘立即清空。
- **连续超时自动熔断（`CIRCUIT_BREAKER_TRIP_COUNT = 3`）**：同一脚本连续超时 `3` 次自动熔断停用并降级回默认布局或原始报文，后续调用不触发 VM 直接返回错误；界面须提供非阻塞提示条与一键「重试启用 / Retry script」恢复入口（保存编辑后亦自动复位）。
- **UI 防抖与零卸载重绘**：
  - 窗口拖拽缩放（`120 ms`）与内置脚本编辑器输入（`180 ms`）必须合并防抖，且缩放复用内存源码不再反复读盘。
  - [`layout_render.rs`](../../app/src/ui/layout_render.rs) 必须分离 size-independent 的 `LayoutTopology` 与 `slot_sizes` 响应式信号：**仅窗格尺寸改变而拓扑不变时严禁触发 `dynamic(...)` 卸载重建 `DataTable` / `Editor`**。

---

## 5. 零 I/O 纯函数与原生编解码桥接

- **单文件隔离沙箱**：严禁网络、任意文件系统读写、环境变量、异步定时器、`eval` / `Function` 构造器与外部包导入；初始化时冻结 `globalThis` 与全量内置原型（`Object` / `Array` / `Function` / `String` / `Number` / `Boolean` / `RegExp` / `Map` / `Set`），用户脚本在 `"use strict"` IIFE 中求值，杜绝跨次运行污染全局状态。
- **重 CPU 字节操作桥接 Rust**：后续自定义包转换中涉及 UTF-8/Hex/Base64 编解码、内层 `gzip`/`zstd`/`brotli` 解压或 Protobuf/MsgPack 辅助解析时，必须通过 `zniff/*` 宿主函数直接桥接至 `zniff` 既有 Rust 依赖执行，禁止在 JS 循环里手写慢速解压或引入重复第三方 crate。

---

## 6. 性能基准与回归门禁

每次修改 `script_worker.rs`、`layout_runtime.rs`、`layout_tsx.rs`、`layout_spec.rs`、`layout_render.rs`、`layout_settings*.rs` 或未来包转换脚本引擎后，必须执行并核对：

1. `make bench`（`app/benches/large_session.rs`）：确认 `10k` 流会话指标未回退，且 `qjs:warm` 显著低于 `qjs:cold`（热复用通常 `< 0.5 ms`）。
2. `make test`：通过全部脚本生命周期、熔断、原型隔离、无扫盘启动、`DataTable` 零重建与依赖树防膨胀单测（含 `cargo_manifest_and_lockfile_guard_against_heavy_js_deps`）。
3. `make fmt && make lint` + `wasm32` `clippy` 与 `make check-wasm` + `--no-default-features` 全 0 警告。
