---
trigger: always_on
---

# zniff 抓包内核、会话与系统代理约束

- **技术选型边界**：禁止引入 `hudsucker`、`HTTP/3`、`QUIC`、通用透明代理（除桌面内置按进程透明代理扩展）、`SQLite`、`cyberapi`、`Firefox` 证书库。连接转发基于 `hyper` + `rustls` + `tokio`（核心已拆为 `app/src/proxy/*.rs` 与 `app/src/local_proc.rs`）。
- **TLS 与协议跟随**：
  - 按客户端 ClientHello ALPN 协商源站协议，不强行降级 HTTP/1.1，不合并 `Cookie`，不擅改业务请求/响应头大小写。
  - TLS 探测失败时必须关闭当前连接、新开 TCP 再放行原始 ClientHello（不得复用已探测失败的 TCP）；ServerHello 之前失败可退回纯 TCP 隧道（仅当打开「隧道失败」开关才写入列表）；ServerHello 之后失败只能记错，不可退回隧道。
- **内存会话与盘溢环**：
  - Body 与 WebSocket/SSE 均为旁路记录，不阻塞转发主链路。单条正文上限 1 MB（`BODY_CAP`），未星标超过 `INLINE_BODY_CAP`（16 KB）自动写入临时 `zniff-spill/` 环形分片（Unix 打开即 unlink 匿名化），点详情/导出/重放/`body:` 时 `pread` 按需读取；内存总上限 `SESSION_CAP`（256 MB），星标 `PIN_CAP`（32 条）不参与淘汰。
- **准入与规则安全**：
  - `allow_hosts`（含 `--strict-allow-hosts` / `ZNIFF_STRICT_ALLOW_HOSTS`）拦截所有代理目标与 `/proxy.pac`，豁免 `/zniff`、`/zniff-ca.crt`、`/proxy.pac`、`/mcp`、`/s/`、`/relay` 本地路由；`hook` 仅允许向本机回环地址发起调用；静态目录 `map-local` 严格防 `..` 与软链目录越界。
- **根证书与系统代理安全**：
  - 根 CA 名为 `zniff CA`（移除同时清理旧名 `Snoop CA`）；退出程序不卸载根 CA；真机 CA 与 Wi-Fi 代理仅由用户操作，桌面「安装证书」仅影响本机或明确点击的 `Booted` iOS 模拟器（`xcrun simctl` 退出时恢复模拟器代理）。
  - `use_system_proxy` 默认关，仅在开启且处于抓包中改动，现场独立记在 `proxy-snapshot.toml`，启动时通过 `restore_if_dirty()` 恢复，停止抓包与测试期间绝不能污染或遗留系统代理配置。
- **发布与更新校验**：
  - 桌面端 `.app` 打包通过 `nsh package` + `.github/workflows/sign_and_package_macos.sh --embed-sysext` 完成，CLI/服务端通过 `--no-default-features`（`make release-cli`）构建；更新下载必须强制校验 `SHA256SUMS`。
