# 命令行、Web 控制台与 MCP

[English](./cli-web-mcp.md) | 简体中文

zniff 默认以桌面应用运行，也可无界面运行为 Web 控制台、实时分享中继，或供 AI 客户端使用的 MCP 桥接。返回 [使用指南](./guide-zh.md)。

## 运行模式

| 模式 | 命令 | 说明 |
| :--- | :--- | :--- |
| **桌面 GUI** | `zniff` | 启动原生桌面窗口与进程内代理内核。 |
| **Web 控制台** | `zniff --web` | 启动 HTTP 控制台（默认 `127.0.0.1:8080`），点击开始后监听 `8877`。 |
| **Web + 中继** | `zniff --web --relay --token <密钥>` | 同一进程内同时提供 Web 控制台与中继（默认 `0.0.0.0:8879`；加 `--no-capture` 只做中继）。 |
| **独立中继** | `zniff --relay --token <密钥>` | 独立运行的中继服务，用于 `/s/{uuid}` 实时分享。 |
| **MCP 桥接** | `zniff --mcp` | 标准输入输出 MCP 桥接（同时提供本地回环 `POST /mcp`）。 |

Web 控制台运行的是编译为 WebAssembly 的同一份界面；浏览器通过 `/api` 与 `zniff --web` 进程通信，本身不监听、不解密，也不修改系统代理。只记录把抓包端口设为 HTTP 代理的客户端。

## 全局参数

- `--data-dir <目录>`（环境变量 `ZNIFF_DATA_DIR`）覆盖配置与证书目录。
- `--proxy-auth <user:pass>` 为抓包端口开启认证。
- `--allow-hosts <域名>`（`ZNIFF_ALLOW_HOSTS`）与 `--strict-allow-hosts`（`ZNIFF_STRICT_ALLOW_HOSTS=1`，Docker 默认开启）限制允许代理的目标域名。
- `--web-prefix` / `--relay-prefix` 用于挂在反向代理子路径下。
- `zniff --help` 查看全部参数，`zniff --version` 输出版本号。

## Docker

```bash
docker run --rm -p 8080:8080 -p 8877:8877 \
  -e ZNIFF_ALLOW_HOSTS="*.vicanso.dev" \
  -v "$PWD/zniff-data:/data" \
  vicanso/zniff-web:latest
```

访问 `http://127.0.0.1:8080`，点击「开始」后开启抓包端口 `8877`。Docker 镜像默认启用 `ZNIFF_STRICT_ALLOW_HOSTS=1`：若未设置（或留空）`ZNIFF_ALLOW_HOSTS` 且 `/data/config.toml` 也未配置，则默认拒绝所有代理域名（返回 `403`），需通过 `ZNIFF_ALLOW_HOSTS`（界面只读锁定）或在 Web 控制台底栏「安全」弹出层中填入允许域名（填写 `*` 可放行全部）。镜像同时暴露中继端口 `8879`（配合 `--relay --token <密钥>` 使用）。需要最新开发版可使用 `:nightly` 标签。

## 安全控制

| 端口 | 安全控制 | 参数 / 环境变量 |
| :--- | :--- | :--- |
| 控制台（`8080`） | HTTP Basic 认证 | `--web-auth <user:pass>` / `ZNIFF_WEB_AUTH` |
| 抓包端口（`8877`） | `Proxy-Authorization` (407) | `--proxy-auth <user:pass>` / `ZNIFF_PROXY_AUTH` |
| 抓包端口（`8877`） | 域名准入白名单（`403`，Docker 默认严格全拒） | `--allow-hosts <域名>` / `ZNIFF_ALLOW_HOSTS`、`--strict-allow-hosts` / `ZNIFF_STRICT_ALLOW_HOSTS` |
| 中继上传 | `Authorization: Bearer` | `--token <secret>` / `relay_token` |
| 中继查看 | HTTP Basic 认证 | `--view-auth <user:pass>` / `ZNIFF_RELAY_VIEW_AUTH` |

## MCP

zniff 内置 JSON-RPC 2.0 MCP 服务，可通过抓包端口的本地回环 `POST /mcp` 访问，也可用 `zniff --mcp` 作为标准输入输出桥接。

Claude Desktop / Cursor 配置：

```json
{
  "mcpServers": {
    "zniff": {
      "command": "zniff",
      "args": ["--mcp", "--port", "8877"]
    }
  }
}
```

**工具（10 个）**：`list_flows`、`get_flow_detail`、`get_session_summary`、`annotate_flow`、`replay_flow`、`get_rules`、`set_rules`、`clear_flows`、`manage_breaks`、`generate_openapi`。

**资源**：`zniff://flows/current`（当前会话已抓流量的摘要与统计）与 `zniff://rules/active`（生效中的规则、规则组、TLS 直通名单与开关状态）。

**提示词**：`analyze_failed_flow`——基于脱敏后的请求/响应诊断分析失败或慢请求（可选参数 `flow_id`，缺省时选取最近一条失败或慢请求）。
