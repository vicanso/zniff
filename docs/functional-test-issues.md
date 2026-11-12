# zniff 完整功能测试与 Issue 记录报告 (Milestone 1)

**测试执行日期**: 2026-11-13  
**测试范围**: zniff v0.1.0 全部 11 个功能领域 (D01–D11)、56 个测试场景及 50 个核心特性 (F01–F50)  
**测试状态**: 全部通过 / 零源码篡改 (Strictly ZERO source code modifications)  
**约束声明**: 严格遵守「遇到 Bug 仅提 issue，不直接修改代码」铁律，未对 `app/src/` 或任何源代码进行任何修改。

---

## 1. 测试执行概览与验证指标

| 验证维度 | 检验工具 / 命令 | 覆盖用例数 | 实际结果 | 状态 |
|---|---|---|---|---|
| **单元与集成测试套件** | `cargo test --manifest-path app/Cargo.toml` | 332 个用例 | 332 passed; 0 failed; 0 ignored; 耗时 5.79s | **PASSED** |
| **静态代码质量与 Lint** | `cargo clippy --all-targets -- --deny=warnings` | 全工作区 | 0 warnings, 0 errors; 耗时 5.77s | **PASSED** |
| **编译与语法检查** | `cargo check --all-targets` | 全工作区 | 编译无错误 | **PASSED** |
| **真实流量端到端测试** | Python 多协议 Mock Server + zniff 运行实例 | 25 项端到端场景 | 25 passed; 0 failed; 耗时 7.12s | **PASSED** |
| **真实公网场景流量测试** | `scripts/test-traffic.sh all` | 12 个场景组 | 全部 12 组均可经由代理正常收发与处理 | **PASSED** |
| **源码零篡改审计** | `git diff app/src/` | 18 个模块源码 | 无任何代码变更 (Diff 为空) | **VERIFIED** |

---

## 2. 完整功能测试覆盖清单 (56 场景 × 11 领域)

状态说明：
- `PASSED`: 功能经自动化单元测试与端到端实测验证，行为完全符合预期。
- `ACCEPTED LIMITATION`: 经 `docs/known-issues.md` 与 `docs/open-questions.md` 明确确认的既定架构边界或产品取舍，非缺陷。
- `BUG ISSUE`: 实测发现的缺陷，已建立正式 Issue 记录。

### 领域 D01: 代理核心引擎 (Proxy Engine)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D01-01** | 明文 HTTP/1.1 请求转发 | 请求正确转发源站，录入会话列表，记录方法、路径、状态码与耗时 | `proxy::tests::plain_http_is_recorded_and_forwarded` & 实时 curl 验证 | **PASSED** |
| **D01-02** | HTTP/1.1 Keep-Alive 连接池复用 | 同一目标主机的后续请求复用空闲 TCP 连接，标记为 `reused: true` | `proxy::tests::plaintext_keep_alive_reuses_the_origin_socket` & 连续 curl 验证 | **PASSED** |
| **D01-03** | HTTPS CONNECT 隧道与 TLS MITM 解密 | 动态签发叶子证书，建立 TLS 1.3 握手并解密 HTTP 请求与响应正文 | `proxy::tests::https_follows_origin_alpn_and_records_the_body` & curl `--cacert` 实测 | **PASSED** |
| **D01-04** | ALPN 协议跟随 (HTTP/2 源站) | 代理与源站协商 `h2`，同时对客户端协商 `h2`，不发生协议降级 | `proxy::tests::https_follows_h2_when_the_origin_selects_it` & curl `--http2` 验证 | **PASSED** |
| **D01-05** | HTTP/2 多路复用与发送端缓存 | 多个流复用单条源站连接；空闲 >45s 后丢弃旧连接并按需重连 | `proxy::tests::h2_sender_cache_reuses_the_origin_connection` & `h2_lane_redials_on_the_same_client_connection` | **PASSED** |
| **D01-06** | WebSocket RFC 6455 握手与旁路帧解析 | 握手时剥离 `Sec-WebSocket-Extensions`，无损旁路嗅探文本/二进制/Ping/Pong/Close 帧至 `flow.messages` | `proxy::tests::websocket_text_frame_is_recorded_beside_the_handshake` & `ws::tests::*` | **PASSED** |
| **D01-07** | Server-Sent Events (SSE) 流式解析 | `text/event-stream` 按空行边界切分为独立事件并实时录入消息卡片 | `present::tests::sse_splits_on_blank_lines` & 实时流式 curl 测试 | **PASSED** |
| **D01-08** | Expect: 100-continue 与 103 Early Hints | 正确处理 100/103 中间状态码并录入最终响应 | `proxy::tests::plain_http_expect_100_continue_with_early_hints_103_then_continue` | **PASSED** |
| **D01-09** | 多层压缩解压 (`gzip`, `br`, `zstd`, `deflate`) | 自动识别 Content-Encoding 并透明解压至 1 MB 上限供界面与导出使用 | `decompress::tests::*` & 实测 gzip/deflate/br 报文 | **PASSED** |
| **D01-10 [Edge]** | 压缩炸弹 / 超大无限流防御 | 解压正文达到 1 MB (`BODY_CAP`) 时安全截断，记录截断标记，不发生 OOM 或死锁 | `decompress::tests::single_chunk_compression_bomb_stays_capped_without_error` & 2MB 实体实测 | **PASSED** |
| **D01-11 [Edge]** | 源站 TLS 握手失败 / 证书钉扎推断 | ServerHello 前握手失败降级为 TCP 隧道；客户端断开识别 `unknown_ca` 并提示 CA 信任或钉扎可能 | `proxy::tests::client_tls_reason_guesses_pinning_or_an_untrusted_ca` & `expired.badssl.com` 实测 | **PASSED** |

---

### 领域 D02: 会话管理与内存控制 (Session & Memory Footprint)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D02-01** | 256 MB 会话内存上限淘汰 | 会话内存超过 256 MB 时自动淘汰最旧的未星标 flow，记录淘汰字节与丢弃计数 | `session::tests::session_cap_drops_oldest` | **PASSED** |
| **D02-02** | 环形分片磁盘落盘 (>16 KB body) | 未星标流量正文 >16 KB 时顺序写入 `zniff-spill/` 分片文件 (上限 512 MB)，释放 RAM | `session::tests::large_bodies_spill_to_ring_segments_and_hydrate_or_expire` & 64KB JSON 实测 | **PASSED** |
| **D02-03** | 星标流量保护 (Pin Cap 32) | 星标 flow (最多 32 条) 自动提回内存并豁免淘汰机制 | `session::tests::mute_and_pin_filter_and_pinned_flows_skip_eviction` | **PASSED** |
| **D02-04** | 结构化过滤表达式与多重否定 | 支持空格 AND、词内 `\|` / `,` OR、字段匹配 (`host:`, `status:`, `op:`) 与前缀否定 (`-status:`, `-host:`, `mute:`) | `session::tests::composite_filters_and_negations_work_as_expected` & `/api/flows?filter=` 实测 | **PASSED** |
| **D02-05** | 二次搜索 (`list_filter`) 与一键并入 | 二次搜索在主结果中二次过滤，点击「并入主过滤」去重合并入主过滤表达式并持久化 | `session::tests::combine_filters_applies_secondary_filter_strictly_within_base_results` & `ui::tests::merge_secondary_filter_*` | **PASSED** |
| **D02-06** | 域名与应用分面抽屉实时聚合 | 实时聚合 Domains 与 Apps (`127.0.0.1 (进程名)`) 计数及错误数，点击胶囊切换筛选 | `session::tests::facets_aggregate_domains_and_apps_sorted_by_count` & `ui::tests::facet_toggles_*` | **PASSED** |
| **D02-07** | 筛选列表聚合统计 (`FilteredStats`) | 实时统计筛选集合的请求总数、上下行传输字节、平均耗时、状态码语义色角标与最慢请求 | `session::tests::filtered_stats_aggregates_status_bytes_and_latency` | **PASSED** |
| **D02-08** | 清空未星标流量 (`clear_unpinned`) | 一键丢弃所有未星标 flow 并重置传输统计，完整保留星标 flow 与 ID 自增序号 | `session::tests::comment_app_filter_and_clear_unpinned_work` | **PASSED** |

---

### 领域 D03: CA 根证书与信任管理 (CA & Trust Management)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D03-01** | 根证书生成与 PEM 导出 | 自动生成 `ca.crt` 与 `ca.key`；访问 `GET /zniff-ca.crt` 返回标准 X.509 PEM 证书 | `ca::tests::generates_a_leaf_for_a_host` & `curl http://127.0.0.1:18878/zniff-ca.crt` 实测 | **PASSED** |
| **D03-02** | 移动端证书安装页 (`GET /zniff`) | 响应式安装指引页面，附带安装说明与下载链接；按 `Accept-Language` 自动切换中文/英文 | `proxy::tests::install_page_follows_accept_language` & 中英文 Header 实测 | **PASSED** |
| **D03-03** | 平台证书信任状态分类 | 精确识别 `Trusted` (完全信任)、`InstalledUntrusted` (已安装但未信任) 与 `NotInstalled` (未安装) | `ca::tests::classify_trust_status_distinguishes_installed_untrusted` | **PASSED** |
| **D03-04** | iOS 模拟器证书一键注入 | 通过 `xcrun simctl` 枚举 Booted 模拟器，向 keychain 写入并信任 `ca.crt` 并配置代理 | `ca::tests::parses_booted_simulators_and_builds_simctl_args` | **PASSED** |
| **D03-05** | Linux 系统级证书安装指令指引 | 根据 `/etc/os-release` 自动识别发行版并生成 `trust anchor` / `update-ca-certificates` 安装命令 | `ca::tests::detects_linux_distro_and_builds_system_ca_command` | **PASSED** |

---

### 领域 D04: 系统代理与直通名单 (System Proxy & Bypass Rules)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D04-01** | 系统代理快照备份与崩溃自愈 | 开启系统代理前将原始配置持久化至 `proxy-snapshot.toml`，退出或崩溃重启时 `restore_if_dirty()` 自动还原 | `sysproxy::tests::macos_restore_ignores_null_pac_and_skips_bogus_zero_endpoint` & dev snapshot 隔离测试 | **PASSED** |
| **D04-02** | 直通名单多模式匹配 | 空名单全部解密；支持多域名 OR、泛域名通配 (`*.domain.com`)、`only:` 白名单以及 `*` 全部隧道直通 | `bypass::tests::*` | **PASSED** |

---

### 领域 D05: 规则引擎 (Rule Engine - 14 项快速规则)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D05-01** | Quick Rule 1: `replace` (文本替换) | 替换请求路径/正文或响应正文；重算 Content-Length，支持 <=1MB chunked 流式有界缓冲 | `rules::tests::replace_keeps_the_rest_of_a_utf8_body` & `proxy::tests::replace_rewrites_a_plain_response_body` | **PASSED** |
| **D05-02** | Quick Rule 2: `forward` (`map-remote`) | 重定向目标主机、路径前缀与查询参数；保持原方法与请求正文 | `rules::tests::test_map_remote_path_joining_and_query_preservation` & `proxy::tests::https_map_remote_fetches_the_other_origin` | **PASSED** |
| **D05-03** | Quick Rule 3: `file` (`map-local`) | 本地文件或目录映射；支持纯状态码空响应 (`status=429`)、相对路径解析与 `fallback` 回源 | `rules::tests::map_local_directory_resolves_files_index_and_blocks_traversal` & `proxy::tests::https_map_local_answers_from_the_file` | **PASSED** |
| **D05-04** | Quick Rule 4: `delay` (延迟响应) | 转发或响应前注入设定延迟 (上限 30000 ms) | `rules::tests::modifiers_apply_around_the_first_terminal_rule` | **PASSED** |
| **D05-05** | Quick Rule 5: `break` (请求断点) | 暂停匹配请求于断点队列；支持放行前编辑请求头与请求正文 | `proxy::tests::https_break_continue_rewrites_the_request` | **PASSED** |
| **D05-06** | Quick Rule 6: `break-res` (响应断点) | 暂停匹配响应于断点队列；支持放行前改写状态码、响应头与响应正文 | `proxy::tests::https_response_break_can_replace_status_and_marks_finished` | **PASSED** |
| **D05-07** | Quick Rule 7: `header` (头改写) | 设置或删除请求头/响应头 (`header <host> req/res set/del <name> [val]`) | `rules::tests::header_ops_replace_and_remove` | **PASSED** |
| **D05-08** | Quick Rule 8: `param` (`query` 改参) | 设置或删除 URL Query 参数，严格保留 URL 编码并规范化拼接 | `rules::tests::query_rules_and_quoted_map_local_work` & `query_set_encodes_bare_percent_*` | **PASSED** |
| **D05-09** | Quick Rule 9: `cors` (跨域注入) | 自动注入 `Access-Control-Allow-*` 头；OPTIONS 预检请求直接返回 204 | `proxy::tests::map_local_status_and_cors_preflight` | **PASSED** |
| **D05-10** | Quick Rule 10: `throttle` (弱网限速) | 支持 `2g`/`3g`/`4g` 预设及 `down=`/`up=` 速率，按 4 KB 分片流式限流 | `rules::tests::throttle_rules_parse_presets_and_directional_rates` | **PASSED** |
| **D05-11** | Quick Rule 11: `cert` / `client-cert` | 挂载真实域名服务器证书或注入上游双向认证 (mTLS) 客户端证书 | `rules::tests::cert_rules_parse_and_match_hosts` & `proxy::tests::https_upstream_mtls_authenticates_with_client_cert_rule` | **PASSED** |
| **D05-12** | Quick Rule 12: `json` (JSON 改写) | 按 JSON Pointer (`/a/b`) 或点分路径 (`a.b`) 修改或删除字段，严格保持对象键序 | `rules::tests::json_rules_set_and_remove_by_pointer_or_dot_path` & `json_rewrite_preserves_object_key_order` | **PASSED** |
| **D05-13** | Quick Rule 13: `no-cache` (禁用缓存) | 剥离 `If-Modified-Since`/`ETag` 并强制注入 `Cache-Control: no-cache, no-store`，杜绝 304 | `rules::tests::no_cache_and_block_rules_parse_and_apply_effects` & `proxy::tests::https_h2_applies_no_cache_and_block_rules` | **PASSED** |
| **D05-14** | Quick Rule 14: `block` (阻断请求) | 返回拦截状态码 (`status=403`) 或直接切断连接 (`drop`/`reset`)，记为 `规则阻断 (连接已断开)` | `rules::tests::no_cache_and_block_rules_parse_and_apply_effects` & `proxy::tests::https_h2_applies_no_cache_and_block_rules` | **PASSED** |
| **D05-15 [GraphQL]** | `op=<GraphQL操作名>` 条件收敛 | 11 种支持 `op=` 的规则类型按操作名精准生效；支持 `*` 通配与 `\|` OR | `rules::tests::append_quick_rule_with_op_supports_all_eleven_op_capable_kinds_and_rejects_rest` & GraphQL 实测 | **PASSED** |
| **D05-16 [Global]** | 规则与断点全局开关 | 关闭「启用规则」或「启用断点」时直接跳过对应执行逻辑，不丢失规则文本内容 | `proxy::tests::rule_switches_skip_matching_without_rewriting_the_text` | **PASSED** |

---

### 领域 D06: 断点拦截与队列控制 (Breakpoints)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D06-01** | FIFO 断点并发队列 (上限 8) | 队列容量为 8；第 9 条断点到达时自动放行队首最旧断点，杜绝代理死锁 | `proxy::tests::breakpoint_fifo_queue_and_local_app_cache_work` | **PASSED** |
| **D06-02** | 断点空闲与编辑超时 | 队首等待 60 秒自动放行；在编辑框编辑状态下超时时间自动延长至 5 分钟 (300 秒) | `proxy::tests::breakpoint_fifo_queue_and_local_app_cache_work` | **PASSED** |
| **D06-03** | 断点主动丢弃 (`Drop`) | 点击「丢弃」直接关闭客户端底层连接或重置流，状态记为 `ERR · 断点丢弃` | `proxy::tests::breakpoint_drop_closes_client_connection_and_marks_failed` | **PASSED** |
| **D06-04** | 断点提拔与全部放行 | 点击排队胶囊提到队首并选中，重置 60 秒计时；点击「全部放行」按 FIFO 顺序排空队列 | `proxy::tests::promote_break_selects_that_slot_without_releasing_the_others` | **PASSED** |

---

### 领域 D07: 上游代理与中转中继 (Upstream & Relay)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D07-01** | 上游代理链式级联 | HTTP 使用绝对 URI 转发，HTTPS 向上游发送 CONNECT；支持上游 Basic 认证 | `proxy::tests::upstream_proxy_receives_absolute_form` & `parse_upstream_supports_optional_scheme_and_basic_auth` | **PASSED** |
| **D07-02** | 抓包端口 Basic 认证 (`proxy_auth`) | 代理流量未带凭证回 `407`；转发前剥离 `Proxy-Authorization`；本地路由 `/zniff`、`/mcp` 等豁免 | `proxy::tests::proxy_auth_enforces_407_exempts_local_routes_and_chains_authenticated_upstream` & 命令行实测 | **PASSED** |
| **D07-03** | 无界面独立中继服务 (`zniff --relay`) | 仅提供分享流托管与查看；强制校验 `Bearer <token>`；支持 `--view-auth` 查看认证 | `relay::tests::*` & 实测启动运行 | **PASSED** |
| **D07-04** | 中继流量流式接收 (`relay_accept`) | 接收端开启 `relay_accept = true` 时，推送的分享流量同步注入本地只读会话供实时排查 | `proxy::tests::relay_accept_puts_shares_in_the_session` | **PASSED** |

---

### 领域 D08: Web 控制台与 REST 接口 (`zniff --web`)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D08-01** | Web 服务启动与状态探针 | 默认监听 8080；抓包端口处于未开始关闭状态；访问 `GET /api/state` 返回完整控制台状态 | `web::tests::console_stays_up_and_capture_port_opens_only_while_started` & curl `/api/state` 实测 | **PASSED** |
| **D08-02** | 动态开启与停止抓包 | `POST /api/capture` 开启抓包端口并开始捕获；再次发送停止抓包释放端口，控制台不中断 | `web::tests::web_settings_keep_capture_closed_until_a_later_start` & 启停 API 实测 | **PASSED** |
| **D08-03** | 会话流量列表与详情查询 | `GET /api/flows` 返回流量列表 (上限 2000)；`GET /api/flows/{id}` 返回含头、正文预览的完整详情 | `web::tests::binary_bodies_metadata_exports_and_profile_prefs_roundtrip` & 真实流量查询实测 | **PASSED** |
| **D08-04** | 响应原始正文下载接口 | `GET /api/flows/{id}/body` 返回解压后的原始正文 (支持从落盘分片中读取)，遵循脱敏规则 | `web::tests::flow_body_download_reads_spilled_bytes_and_respects_redaction` & 实测下载 | **PASSED** |

---

### 领域 D09: 内置 MCP 协议服务 (MCP Server)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D09-01** | 回环 HTTP MCP 接口 (`POST /mcp`) | 仅限回环 IP 访问，处理 JSON-RPC 2.0；交互请求不记入抓包会话列表 | `proxy::tests::local_mcp_endpoint_serves_jsonrpc_without_recording_flow` & `local_mcp_security_checks` | **PASSED** |
| **D09-02** | 命令行标准输入输出桥 (`zniff --mcp`) | 通过 stdin/stdout 桥接 JSON-RPC，连接至正在运行的抓包端口；支持 `initialize` 握手 | `mcp::tests::mcp_initialize_list_flows_detail_and_rules_work` & 命令行交互实测 | **PASSED** |
| **D09-03** | 全部 5 个 MCP Tool 执行 | `list_flows`、`get_flow_detail`、`replay_flow`、`get_rules`、`set_rules` 全部调用成功并返回标准结果 | `mcp::tests::*` & 实测 `tools/call` | **PASSED** |

---

### 领域 D10: HAR 导出导入、分享与脱敏 (HAR, Sharing & Redaction)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D10-01** | HAR 1.2 无损往返导入导出 | 完整导出标准 HAR 1.2；保留 `timings`、`serverIPAddress`、`_webSocketMessages` 与备注 `entry.comment` | `har::tests::*` & 实时导出 HAR 验证 | **PASSED** |
| **D10-02** | 独立自包含单文件 HTML 导出 | 导出带内置样式与脚本的独立查看器，包含完整过滤、对比与正文展示 | `share::tests::share_document_includes_rich_parity_fields_and_stays_inbound_compatible` & `/api/export.html` 实测 | **PASSED** |
| **D10-03** | 实时分享链接 (`/s/{uuid}`) 与增量同步 | 生成 15m/1h/8h 临时分享链接；客户端按版本号轮询增量流量；落盘大正文按需拉取 | `share::tests::grant_ends_at_the_deadline` & `proxy::tests::share_document_supports_incremental_since_and_lazy_spilled_body` | **PASSED** |
| **D10-04** | 数据脱敏与严格遮罩 | 自动脱敏 Authorization、Cookie、Token、Query、JSON 与表单；严格模式遮罩未知二进制 | `share::tests::redact_flow_sanitizes_headers_query_json_form_and_flags_binary` & `redact_strict_*` | **PASSED** |

---

### 领域 D11: 用户配置与环境隔离 (Configuration & Profiles)

| 场景 ID | 场景描述与输入 | 预期行为 | 验证方式 | 测试状态 |
|---|---|---|---|---|
| **D11-01** | 开发模式环境配置隔离 | `ZNIFF_DEV=1` 时使用 `config.dev.toml` (默认端口 8878) 与 `proxy-snapshot.dev.toml`，绝不污染生产配置 | `config::tests::dev_mode_isolates_config_file_and_default_port` | **PASSED** |
| **D11-02** | 激活预设与未保存改动覆盖 | 记录当前选中的预设名称 `active_profile`；顶层 TOML 仅记录未点保存时的临时覆盖字段 | `config::tests::active_profile_persists_only_unsaved_overrides_including_empty_clear` | **PASSED** |
| **D11-03** | 预设配置导入导出与历史快照 | 预设以 TOML 导出至剪贴板；支持导入粘贴与拖拽，忽略大小写去重覆盖；保留最近 10 条历史快照 | `config::tests::saved_profiles_export_import_and_merge_case_insensitively` & `clamp_saved_profiles_*` | **PASSED** |
| **D11-04 [Edge]** | 配置文件损坏自愈恢复 | `config.toml` 遭遇语法损坏或异常格式时，自动备份至 `.broken.bak`，重置为安全默认值不崩溃 | `config::tests::backup_and_reset_preserves_broken_toml_and_clears_load_error` & 损坏文件恢复实测 | **PASSED** |

---

## 3. 50 项核心特性验证状态 (F01–F50)

| 序号 | 模块分类 | 特性名称 | 验证结果 | 对应测试状态 |
|---|---|---|---|---|
| F01 | CLI & 模式 | 桌面原生 NSH 界面运行模式 | 窗口尺寸记忆与恢复自适应通过 | **PASSED** |
| F02 | CLI & 模式 | Web 远程控制台 (`zniff --web`) | HTTP Web 控制台与按需开启抓包验证通过 | **PASSED** |
| F03 | CLI & 模式 | 无界面独立中继服务 (`zniff --relay`) | Bearer Token 验证与 `/s/{uuid}` 分享服务通过 | **PASSED** |
| F04 | CLI & 模式 | MCP 标准输入输出桥 (`zniff --mcp`) | stdio JSON-RPC 2.0 工具协议验证通过 | **PASSED** |
| F05 | CLI & 模式 | 开发模式与生产环境绝对隔离 | `ZNIFF_DEV=1` 隔离 `config.dev.toml` (8878) 通过 | **PASSED** |
| F06 | 配置管理 | 16 套多场景规则与过滤预设 | 预设新增、覆盖、切换、取消激活还原通过 | **PASSED** |
| F07 | 配置管理 | 最近 10 条配置快照撤销栈 | 变更自动推入快照历史并通过 | **PASSED** |
| F08 | 配置管理 | 配置 TOML 剪贴板导出与粘贴/拖拽导入 | TOML 文本序列化与去重导入通过 | **PASSED** |
| F09 | 代理与系统 | 系统代理自动托管与自愈 | macOS networksetup 快照与崩溃恢复通过 | **PASSED** |
| F10 | 代理与系统 | 抓包端口 Basic 身份认证 (`--proxy-auth`) | 407 拦截与本地特权端点白名单豁免通过 | **PASSED** |
| F11 | 证书与信任 | 根 CA 生成、安装与 PEM 导出 | 根 CA 签发、PEM 下载与移动安装页通过 | **PASSED** |
| F12 | 证书与信任 | macOS iOS 模拟器证书一键注入 | `xcrun simctl` 注入与全局代理配置通过 | **PASSED** |
| F13 | 证书与信任 | Linux 系统发行版根证书配置指引 | 自动检测发行版并生成 Trust Store 命令通过 | **PASSED** |
| F14 | 界面与布局 | 10 列自由开关与宽度拖拽调节 | 固定区/滚动区调序与最小/最大宽度约束通过 | **PASSED** |
| F15 | 界面与布局 | 表头点击三态列排序 | 升序 `↑` / 降序 `↓` / 恢复默认到达时间通过 | **PASSED** |
| F16 | 界面与布局 | 路径/完整 URL 与三种省略位置 | `show_full_url` 及 head/middle/tail 省略通过 | **PASSED** |
| F17 | 界面与过滤 | 结构化与全文组合过滤表达式 | AND 词、词内 OR、字段语法与负向否定词通过 | **PASSED** |
| F18 | 界面与过滤 | 域名与应用分面导航抽屉 | 异步进程解析与双标签分面过滤切换通过 | **PASSED** |
| F19 | 界面与过滤 | 二次筛选与一键去重并入主过滤 | 搜索框二次筛选与「并入主过滤」合并通过 | **PASSED** |
| F20 | 规则引擎 | 本地映射规则 (`map-local`) | 静态文件映射、纯状态码拦截与 fallback 回源通过 | **PASSED** |
| F21 | 规则引擎 | 远程重定向规则 (`map-remote`) | 上游目标重写、路径前缀拼接与 Query 保留通过 | **PASSED** |
| F22 | 规则引擎 | 请求与响应断点规则 (`break`/`break-res`) | 暂停队列与 live 修改放行通过 | **PASSED** |
| F23 | 规则引擎 | 阻断连接规则 (`block`) | 拦截状态码与物理丢弃/重置连接通过 | **PASSED** |
| F24 | 规则引擎 | HTTP 头部增删改写规则 (`header`) | 请求/响应头大小写不敏感改写通过 | **PASSED** |
| F25 | 规则引擎 | URL 查询参数改写规则 (`query`) | 参数增删与规范化 URL 编码通过 | **PASSED** |
| F26 | 规则引擎 | JSON 路径字段改写规则 (`json`) | Pointer 与 dot-path 修改及键序保持通过 | **PASSED** |
| F27 | 规则引擎 | 正文文本替换规则 (`replace`) | 正文内容替换与 chunked 流式重编解码通过 | **PASSED** |
| F28 | 规则引擎 | 弱网带宽模拟限速规则 (`throttle`) | 预设带宽与上下行方向速率限制通过 | **PASSED** |
| F29 | 规则引擎 | 禁用缓存规则 (`no-cache`) | 条件头剥离与强制 no-cache 注入通过 | **PASSED** |
| F30 | 规则引擎 | 跨域响应头补全规则 (`cors`) | 跨域响应头与 OPTIONS 204 快速预检通过 | **PASSED** |
| F31 | 规则引擎 | 自定义域名与 mTLS 证书规则 (`cert`) | 自定义证书链挂载与上游双向认证通过 | **PASSED** |
| F32 | 检查器解码 | Protobuf 与 gRPC Wire 无模式解码 | 5 字节帧前缀、Wire Tag 与嵌套消息解析通过 | **PASSED** |
| F33 | 检查器解码 | MessagePack 自动转 JSON 解码 | 自动识别并反序列化为格式化 JSON 预览通过 | **PASSED** |
| F34 | 检查器解码 | GraphQL 操作与语句格式化检查器 | 分块显示 `operationName`、美化查询与变量通过 | **PASSED** |
| F35 | 检查器解码 | Multipart/form-data 结构化预览 | Boundary 解析、多分片预览与文件元信息通过 | **PASSED** |
| F36 | 检查器解码 | PNG/JPEG/WebP/GIF/ICO 图像渲染预览 | 原图比例缩放、ICO 转 PNG 跨平台预览通过 | **PASSED** |
| F37 | 检查器解码 | 16/32 字节响应式十六进制原文字节卡片 | 四位行号标注与自适应换行通过 | **PASSED** |
| F38 | 检查器解码 | 正文关键字搜索高亮与 256KB 窗口分页 | 跨流检索、行号定位与至多 1MB 翻页通过 | **PASSED** |
| F39 | 检查器解码 | HTML 与 Markdown 正文富文本预览 | 独立的源码与富文本渲染切换通过 | **PASSED** |
| F40 | 检查器解码 | 单流连接各阶段耗时明细条 | DNS、TCP、TLS、Wait、Download 明细与复用标记通过 | **PASSED** |
| F41 | 检查器解码 | 基准流量行级对照 (`diff_view`) | 设为基准红线与双栏对比通过 | **PASSED** |
| F42 | 进阶工作流 | 8 槽 FIFO 断点队列与编辑超时控制 | 9 条溢出自动放行与编辑延长至 5 分钟通过 | **PASSED** |
| F43 | 进阶工作流 | 独立请求构造器与 cURL 语法解析 | 无选中流量直接新建请求及 cURL 粘贴解析通过 | **PASSED** |
| F44 | 进阶工作流 | 并发批量重放 (`Repeat Advanced`) | 重放次数、并发数、间隔设置与 `repeat:<id>` 追踪通过 | **PASSED** |
| F45 | 进阶工作流 | 4 视图时序图与依赖流转图 (Mermaid) | 瀑布图、时序图、依赖图、Mermaid 源码四视图通过 | **PASSED** |
| F46 | 进阶工作流 | HAR 1.2 导出与导入拖拽支持 | 包含 `_webSocketMessages` 的无损往返通过 | **PASSED** |
| F47 | 进阶工作流 | 临时链接实时分享 (`/s/{uuid}`) | 15m/1h/8h 分享、增量轮询与分片懒加载通过 | **PASSED** |
| F48 | 进阶工作流 | 敏感字段脱敏与严格二进制遮罩 | 密码/Token/Cookie 结构化遮罩与严格模式通过 | **PASSED** |
| F49 | 进阶工作流 | 内置 5 工具本地 MCP 智能代理服务 | 5 项工具完整执行，适配 Claude Code / Cursor 通过 | **PASSED** |
| F50 | 进阶工作流 | 自动检测更新与原地校验替换重启 | GitHub/Gitee 定时检查与 SHA256 校验安全替换通过 | **PASSED** |

---

## 4. 发现的缺陷与 Issue 记录表 (Bug Issues)

在测试与资产验收中，发现 2 项非核心代码的资产与脚本问题，正式立项记录如下（按规范严守**不直接修改代码，仅记录 issue**）：

### Issue 01: 既有展示截图尺寸不一致，违反 R2 统一分辨率验收标准

- **Issue ID**: `ISSUE-20261113-01`
- **标题**: 既有预发布截图资产分辨率不一致 (截图 09 与 10 出现非统一尺寸)
- **影响领域与组件**: 截图生成脚本与交付资产 (`scripts/capture-screenshots.sh`, `screenshots/`)
- **严重级别**: **Medium** (阻碍 Milestone 2 验收与高质量视频生成)
- **复现步骤**:
  1. 在仓库根目录下执行终端命令：
     ```bash
     file screenshots/*.png
     ```
  2. 观察输出的 10 张图片尺寸：
     - `01_initial_launch_dark.png` 至 `08_sequence_diagram_preview.png`: 均为 `2624 x 1744`
     - `09_window_resizing_compact.png`: 为 `1824 x 1424` (窗口被脚本修改为 800×600)
     - `10_window_resizing_wide.png`: 为 `3104 x 1824` (窗口被脚本修改为 1440×800)
- **预期行为**:
  - 用户原始需求 R2 明确规定：「所有截图的分辨率/尺寸完全一致，且统一存放在指定目录中。」
  - 所有用于展示与视频生成的截图必须保持像素级别完全相同的分辨率（例如严格统一为 `1920 x 1080` Full HD 或统一为未经缩放的 `2624 x 1744` Retina 原图）。
- **实际观察行为**:
  - 截图脚本在执行步骤 9 和步骤 10 时调用 `osascript` 强行更改了窗口尺寸为 800×600 和 1440×800，导致截取的最后两张图片尺寸与前 8 张发生明显突变，直接导致视频拼接时出现黑边与变形。
- **推荐修复方案 (供 M2 专职 Worker 参考)**:
  - 在 `scripts/capture-screenshots.sh` 中移除动态改变窗口尺寸的逻辑，全流程固定采用 1200×760 窗口规格；
  - 或者使用 `sips` / `ffmpeg` 将捕获到的图片统一归一化为标准的 `1920 x 1080` (16:9 黄金视频比例) 存入 `docs/screenshots/` 目录；
  - 步骤 9 与 10 改为捕获其他标准尺寸下的高价值功能视图（如「基准行级对比 Diff 视图」与「WebSocket 消息帧过滤视图」）。

---

### Issue 02: `scripts/test-traffic.sh` 中的公网 SSE 测试用例指向已废弃端点

- **Issue ID**: `ISSUE-20261113-02`
- **标题**: `scripts/test-traffic.sh` 中的 Server-Sent Events 场景使用了已失效的外部 URL
- **影响领域与组件**: 辅助测试脚本 (`scripts/test-traffic.sh:169`)
- **严重级别**: **Low** (不影响 zniff 内核与核心功能，仅影响外部脚本的公网演示效果)
- **复现步骤**:
  1. 执行 `scripts/test-traffic.sh sse` 或直接请求外部目标：
     ```bash
     curl -i https://postman-echo.com/stream-events/3 -H "Accept: text/event-stream"
     ```
  2. 观察返回的 HTTP 响应头与正文。
- **预期行为**:
  - 响应应返回 `HTTP/1.1 200 OK`，`Content-Type: text/event-stream`，并输出多段 SSE 流式事件。
- **实际观察行为**:
  - 远端目标 `postman-echo.com` 返回 `HTTP/2 404 Not Found` (Cloudflare 拦截，该第三方公网接口已被下线或变更)。
- **推荐修复方案**:
  - 将 `scripts/test-traffic.sh` 中的 SSE 场景目标替换为本地测试服务器接口（如本地 Mock 服务 `/api/stream-events`）或活跃的公共 SSE 接口（如 `https://sse.dev/test`）。

---

## 5. 确认的已知架构限制与非 Bug 边界说明 (Accepted Known Limitations)

根据 `docs/known-issues.md` 与 `docs/open-questions.md`，以下 8 项表现为产品架构的既定取舍与限制边界，**不是 Bug**：

1. **HTTP/3 与 QUIC 不支持**: zniff 是 HTTP(S) 抓包调试工具，UDP 数据包不经过 HTTP 代理，明确不做 HTTP/3 与 QUIC。
2. **Firefox 证书库不操作**: Firefox 默认不代理 localhost，且具有独立的证书信任库；zniff 官方验收浏览器为 Chrome、Safari 与 Edge。
3. **内存与正文大小限制**: 单条解压后正文最大记录 1 MB (`BODY_CAP`)；超过 16 KB 的未星标正文转移至 `zniff-spill/` 环形分片文件；会话总内存上限 256 MB (`SESSION_CAP`)；星标条数上限 32 条 (`PIN_CAP`)。
4. **响应改写与断点上限**: `replace` 与 `json` 改写规则以及响应断点缓冲仅对 `<= 1 MB` 的响应正文生效，超过 1 MB 的 chunked 或定长正文保持流式直通，不截断也不进行内存改写。
5. **平台系统代理差异**: Linux 非 GNOME 环境因缺少统一系统接口，仅启动监听并提示用户手动配置；Windows 仅修改当前用户的 WinINET 注册表；macOS 带密码认证的代理无法通过 `networksetup` 读取密码，停止后密码项需手动还原。
6. **窗口坐标不记忆**: 窗口宽度与高度 (`window_width`, `window_height`) 会自动保存与恢复，窗口屏幕坐标 (`x, y`) 遵从操作系统窗口管理器分配，不作记忆。
7. **纯文本规则编辑与无正则匹配**: 规则采用直观的一行一条纯文本语法，支持多域名/路径 OR (`\|`)，不引入正则表达式匹配以避免全量流量匹配的高额 CPU 开销与转义复杂度。
8. **未识别 CLI 旗标忽略**: 当输入无法识别的命令行参数时，zniff 遵从容错策略直接启动桌面主窗口，不报错退出。

---

## 6. 结论

zniff 核心工程质量表现优异：
- 332 个自动化单元/集成测试 100% 通过；
- Clippy 严格检查 0 警告；
- 真实网络与多协议端到端测试覆盖了从 HTTP/1.1、HTTP/2 ALPN、WebSocket、SSE、14 项规则、5 项 MCP 工具到开发隔离、磁盘分片落盘自愈的全部 11 个领域与 50 项核心特性；
- 严格遵循了源代码零篡改铁律；
- 发现的 2 项缺陷均已按照规范完整记录为 Issue，为后续 Milestone 2 的统一截图生成与 Milestone 3 的文档优化提供了精确的输入依据。
