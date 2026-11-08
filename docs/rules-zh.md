# zniff 规则与配置指南

[English](./rules.md) | 简体中文

本文档详细说明 **zniff** 的配置文件目录、流量过滤语法、TLS 直通名单（Bypass）、9 种转发与改写规则（Rules）、上游代理以及命名配置预设（Saved Setups）的使用方法与示例。

---

## 1. 配置文件与数据目录

`zniff` 的所有持久化配置与根证书均保存在当前操作系统的用户数据目录下（由 [`ca::data_dir()`](../app/src/ca.rs) 解析）：

| 操作系统 | 数据目录路径 |
| :--- | :--- |
| **macOS** | `~/Library/Application Support/zniff` |
| **Windows** | `%APPDATA%\zniff`（例如 `C:\Users\<用户名>\AppData\Roaming\zniff`） |
| **Linux** | `$XDG_DATA_HOME/zniff`（未设置时为 `~/.local/share/zniff`） |

### 目录下的文件说明

| 文件名 | 用途 |
| :--- | :--- |
| `config.toml` | **正式版用户配置文件**（默认监听端口 `8877`）：保存监听设置、界面布局、过滤词、直通名单、规则文本、上游代理与已保存的配置预设（`saved_profiles`）。 |
| `proxy-snapshot.toml` | **正式版系统代理恢复快照**：开启系统代理前备份原系统网络设置；若进程异常退出，下次启动时会自动读取该快照恢复系统代理。 |
| `config.dev.toml` | **开发模式配置文件**（`make dev`、Debug 构建或 `ZNIFF_DEV=1`，默认监听端口 `8878`）：与正式版配置完全隔离，避免开发调试覆盖日常配置。 |
| `proxy-snapshot.dev.toml` | **开发模式系统代理恢复快照**。 |
| `ca.crt` / `ca.key` | 本地生成的 `zniff CA` 根证书与私钥（正式版与开发模式共用）。 |

> **临时大体积正文缓存**：抓包过程中超过 `16 KB` 的已完成非星标请求/响应正文会写入系统临时目录（如 `$TMPDIR/zniff-spill/`）的环形分片文件，退出或过期后自动回收，不在配置目录中落盘。

---

## 2. 流量过滤语法（Traffic Filter）

左栏顶部的过滤输入框与顶栏「过滤」面板左侧区域实时联动，语法为**空格分隔的 `AND` 条件**，单个条件内支持使用 `|` 或 `,` 表示 **`OR`（或）**：

| 语法 / 关键词 | 用途说明 | 示例 |
| :--- | :--- | :--- |
| `host:<域名>` 或 `domain:<域名>` | 按主机名子串或泛域名（`*.`）匹配，支持多域名 `OR` | `host:api.example.com\|*.cdn.com` |
| `path:<路径前缀>` | 按请求路径前缀匹配，支持多前缀 `OR` | `path:/api/v1\|/auth\|/graphql` |
| `ip:<IP>` 或 `client:<IP>` / `app:<进程名>` 或 `proc:<进程名>` | 按客户端来源 IP 或本地进程名匹配，支持多项 `OR` | `ip:127.0.0.1\|192.168.1.25`、`app:curl\|node` |
| `GET` / `POST` / `PUT` / `DELETE` / `CONNECT` 等 | 精确匹配 HTTP 方法（全大写或忽略大小写） | `POST host:api.example.com` |
| `200` / `404` / `1xx`–`5xx` | 精确匹配状态码或状态码段 | `5xx` 或 `404` |
| `失败` / `fail` / `err` / `error` | 筛选发生错误（状态码为 `ERR` 或 `>= 400`）的请求 | `失败` |
| `type:<类型>` 或 `ct:<类型>` | 匹配响应 `Content-Type` 子串 | `type:json`、`ct:html`、`type:image` |
| `body:<关键词>` | 在解压后的请求正文与响应正文中搜索（支持检索落盘的大体积正文） | `body:access_token` |
| `header:<关键词>` | 在请求头与响应头的名称和值中搜索 | `header:authorization`、`header:x-trace-id` |
| `note:<关键词>` 或 `comment:<关键词>` | 按流量备注子串匹配（写 `note:` 可筛选所有带备注的流量） | `note:复现`、`note:` |
| `pin:` / `star:` / `星标` | 仅显示已加星标的请求 | `pin:` |
| `slow:` / `慢` | 仅显示总耗时 `>= 1s` 的慢请求 | `slow:` |
| `mute:<词>` 或 `-<词>` | 排除主机或路径中包含该词的流量，支持 `\|` 分隔多项 | `mute:ads.example.com\|/health` 或 `-telemetry` |
| 普通裸词 | 同时在主机名（Host）、路径（Path）和流量备注中做不区分大小写的子串匹配 | `login` |

**组合过滤示例：**

```text
host:api.example.com|*.staging.test path:/v1|/v2 POST type:json mute:/health
```
含义：匹配域名为 `api.example.com` 或 `*.staging.test`，且路径以 `/v1` 或 `/v2` 开头，方法为 `POST`，响应类型为 `json`，同时排除路径含 `/health` 的请求。

---

## 3. TLS 直通名单（Bypass List / 不解密）

位于顶栏「过滤」弹出层右栏的 **「直通名单 (不解密)」**（对应 `config.toml` 中的 `bypass_hosts`）。
在收到客户端 TLS `ClientHello` 后按 SNI（无 SNI 时按 `CONNECT` 主机名）匹配：

- **默认（空名单）**：拦截并解密全部主机。
- **黑名单模式（默认）**：列出的域名跳过 MITM 解密，直接改为透明 TCP 隧道（列表中记为 `直通名单，未解密`）。适用于启用了证书钉扎（Certificate Pinning）的客户端或系统服务。
- **白名单模式（`only:` 前缀）**：仅解密列出的域名，其余所有域名全部走 TCP 隧道直通。
- **分隔符与通配符**：支持空格、逗号 `,`、分号 `;`、竖线 `|` 分隔；支持精确域名、`*.example.com`（泛子域名，不含裸根域 `example.com`）以及 `*`（全部主机）。

### 直通名单示例

```text
# 示例 1：跳过 Apple 与指定钉扎域名的 TLS 解密
*.apple.com, *.icloud.com | pin.example.com

# 示例 2：白名单模式 —— 只解密 api.example.com 和 *.dev.internal，其余流量全部直通
only: api.example.com *.dev.internal
```

---

## 4. 转发与改写规则（Rules）详解

位于顶栏「过滤」弹出层右栏的 **「转发与改写规则」**（对应 `config.toml` 中的 `rules`）。
编辑器支持**语法实时校验**（有误的行号会高亮提示），修改后**立即对后续新请求生效**，无需重启代理。

### 4.1 通用语法与匹配模式（`<pattern>`）

- **一行一条规则**，`#` 开头或行内 `#` 之后的内容为注释。
- 每条规则的第二个词都是 `<pattern>`（匹配模式，**中间不能有空格**），格式为 `<主机模式>[/<路径前缀>]`：
  1. **主机部分**：
     - 精确域名：`api.example.com`
     - 泛子域名：`*.example.com`（匹配 `a.example.com`、`b.a.example.com`，不匹配 `example.com`）
     - 全部主机：`*`
     - 多域名 OR：用 `|` 或 `,` 连接，如 `api.example.com|*.staging.example.com`
  2. **路径前缀部分**（可选，省略或写 `/` 表示匹配该主机下的所有路径）：
     - 按路径段边界前缀匹配（忽略 `?` 查询参数）：`/api` 会匹配 `/api`、`/api/`、`/api/v1/users`，但**不会**误匹配 `/apix`。
     - 多路径前缀 OR：在第一个 `/` 之后用 `|` 或 `,` 连接，如 `example.com/api|/v2|/graphql`（匹配 `example.com` 下的 `/api`、`/v2` 或 `/graphql`）。
     - 匹配任意主机下的指定路径：`*/api|/v2`。

### 4.2 规则优先级：终点规则 vs 叠加修饰规则

`zniff` 的 9 种规则分为两类：

1. **终点规则（Terminal Rules）**：`map-local`、`map-remote`、`break`、`break-res`
   - 决定请求的最终去向或断点暂停行为。
   - **按从上到下的文本顺序，只有第一条命中的终点规则生效**，后续其他终点规则会被忽略。
2. **叠加修饰规则（Modifier Rules）**：`replace`、`delay`、`header`、`query`、`cors`
   - **不占用终点规则名额**。所有命中的修饰规则都会按顺序叠加生效（即使写在命中的终点规则之后也会生效）。

---

### 4.3 九种规则语法与实战示例

#### ① `map-local` — 本地文件映射 / 纯状态码空响应（Mock 响应）

直接读取本地文件（或在仅指定 `status=<状态码>` 时返回空正文）作为响应返回给客户端，**不连接源站**。响应 `Content-Type` 会根据文件扩展名（`.json`、`.html`、`.js`、`.css`、`.xml`、`.png`、`.jpg`、`.svg`、`.webp`、`.gif`、`.txt` 等）自动推断。

**语法：**
```text
map-local <pattern> [<本地文件路径>] [status=<状态码>] [delay=<时长>]
```
> 注意：若本地文件路径包含空格，请使用英文双引号 `"..."` 包裹；省略 `status=` 时默认返回 `200`；省略 `<本地文件路径>` 时必须提供 `status=<状态码>`（直接返回该状态码与空响应体）；也可直接在列表右键或详情「更多」中点击 **「本地映射」** 将当前响应另存为文件并自动追加规则。

**示例：**
```text
# 将接口直接映射为本地 mock.json（默认 200 OK）
map-local api.example.com/v1/profile ./mocks/profile.json

# 映射包含空格的本地文件路径（使用双引号包裹）
map-local api.example.com/v1/profile "/Users/alice/My Mocks/profile.json" status=200

# 仅返回 503 状态码与空正文，无需创建空文件
map-local api.example.com/v1/maintenance status=503

# 模拟限流或服务端报错（返回 429 状态码并延迟 1.5 秒）
map-local api.example.com/v1/orders /tmp/rate_limit.json status=429 delay=1500ms

# 多环境域名同时使用同一份本地配置
map-local dev.example.com|staging.example.com/config /tmp/app-config.json
```

---

#### ② `map-remote` — 远程地址转发（环境切换 / 路由重写）

将匹配到的请求转发到另一个目标地址（支持 HTTP 与 HTTPS 互转，自动重写 `Host` 头并保留后续子路径与查询字符串）。

**语法：**
```text
map-remote <pattern> <http://或https://目标地址>
```

**路径拼接规则：**
- 当 `<pattern>` 带路径前缀（如 `example.com/api`），且目标地址为 `https://staging.internal:8443/v2` 时：
  请求 `https://example.com/api/users?id=1` 会被转发至 `https://staging.internal:8443/v2/users?id=1`。

**示例：**
```text
# 将线上域名的 /api 请求转发到本地开发服务 http://127.0.0.1:3000/api
map-remote api.example.com/api http://127.0.0.1:3000/api

# 将测试域名整体转发到预发环境 HTTPS 服务
map-remote test.example.com https://staging-api.example.com
```

---

#### ③ `break` — 请求断点（发往源站前暂停编辑）

在请求发往源站之前暂停拦截。界面顶栏会出现黄色断点提示条，右侧详情面板点击「编辑重发」或直接在断点栏点击 **「放行」** / **「丢弃」**：
- 可实时修改 **HTTP 方法、URL、请求头、请求 Body**。
- 点击「丢弃」会直接向客户端返回 `204` 并终止转发；多条断点排队时可点击 **「全部放行」** 一键放行队列中的所有断点。
- 请求断点与响应断点共用最多 **8 条的有界 FIFO 队列**：并发请求命中断点时按先后顺序排队（断点栏显示 `(1 / N)`），不会顶掉正在编辑的请求；队首断点无操作满 60 秒自动放行（正在编辑器中修改内容时会自动续期超时）。

**语法：**
```text
break <pattern>
```

**示例：**
```text
# 拦截支付下单接口，在发往服务端前检查或修改金额/参数
break api.example.com/v1/checkout

# 拦截任意域名下的登录路径
break */auth/login
```

---

#### ④ `break-res`（或 `break-response`）— 响应断点（返回客户端前暂停编辑）

请求正常发往源站，在收到源站完整响应（最多缓冲 `1 MB`）后、返回给客户端之前暂停拦截。
- 可在右侧编辑框中实时修改 **响应状态码（如把 `200` 改为 `500` 或 `403`）、响应头、响应 Body**。
- 点击 **「放行」** 后，修改后的状态码、响应头与正文会立即写回客户端并同步更新列表。

**语法：**
```text
break-res <pattern>
```

**示例：**
```text
# 拦截权限校验接口的响应，手动改状态码或响应 JSON 测试前端容错
break-res api.example.com/v1/permissions
```

---

#### ⑤ `replace` — 文本替换（请求 URL/Body 或响应 Body 替换）

对 UTF-8 文本进行查找替换（叠加规则，可配置多条按顺序执行）：
- **`req`（请求侧）**：同时作用于 **请求目标（`/path?query`）** 和 **请求正文（带 `Content-Length` 且 `<= 1 MB` 的 UTF-8 正文）**。即便请求没有 Body（如 `GET` 请求），也会改写 `/path?query`（改写结果仍须以 `/` 开头）。
- **`res`（响应侧）**：作用于带 `Content-Length` 且 `<= 1 MB` 的 UTF-8 响应正文，替换后自动重算 `Content-Length`。
- `<原文>` 为单个无空格词；`->` 右侧留空表示**删除**匹配到的 `<原文>`。

**语法：**
```text
replace <pattern> req|res <原文> -> <替换为>
```

**示例：**
```text
# 将响应正文中的环境标识从 prod 替换为 debug
replace api.example.com/v1 res "env":"prod" -> "env":"debug"

# 将 GET/POST 请求 URL 查询串或请求体中的 draft=1 替换为 draft=0
replace api.example.com/articles req draft=1 -> draft=0

# 删除请求路径或查询串中的指定片段（箭头右侧留空）
replace api.example.com/api req ?debug=true ->
```

---

#### ⑥ `delay` — 延迟注入（弱网与超时模拟）

在转发请求前注入指定等待时间。多条 `delay` 规则命中同一请求时会累加，单次请求总延迟上限为 **30 秒（`30s` / `30000ms`）**。

**语法：**
```text
delay <pattern> <时长(ms或s)>
```

**示例：**
```text
# 为列表接口增加 2 秒延迟，测试前端骨架屏与 Loading 态
delay api.example.com/v1/feed 2000ms

# 为上传接口增加 5 秒延迟
delay api.example.com/v1/upload 5s
```

---

#### ⑦ `header` — 请求头与响应头增删改

对发往源站的请求头（`req`）或返回客户端的响应头（`res`）执行设置（`set` / `add`，覆盖同名头，不区分大小写）或删除（`del` / `delete` / `rm`）。

**语法：**
```text
header <pattern> req|res set <Header名称> <Header值>
header <pattern> req|res del <Header名称>
```

**示例：**
```text
# 向源站请求注入调试 Header（值支持包含空格）
header api.example.com req set X-Debug-Env staging-gray

# 移除请求中的 Cookie 以模拟未登录态
header api.example.com/public req del Cookie

# 移除响应中的安全限制头或缓存头
header *.example.com res del Content-Security-Policy
header *.example.com res set Cache-Control no-store
```

---

#### ⑧ `query` — URL 查询参数增删改

在转发请求前对 URL 查询参数（Query String）执行设置（`set` / `add`，覆盖同名参数或追加新参数）或删除（`del` / `delete` / `rm`），自动处理 `?`、`&` 分隔符与百分号编码。

**语法：**
```text
query <pattern> set <参数名> <参数值>
query <pattern> del <参数名>
```

**示例：**
```text
# 为所有 /v1 接口自动注入或覆盖 env=staging 参数
query api.example.com/v1 set env staging

# 转发前自动剥离追踪参数 utm_source
query *.example.com del utm_source
```

---

#### ⑨ `cors` — 跨域自动放行（CORS）

为匹配的流量自动补齐跨域响应头（若源站未返回对应头则补充）：
- `Access-Control-Allow-Origin: *`
- `Access-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD`
- `Access-Control-Allow-Headers: *`
- `Access-Control-Max-Age: 86400`

此外，当浏览器的 **`OPTIONS` 预检请求**命中 `cors` 且没有命中其他终点规则时，`zniff` 会**直接返回 `204 No Content` + CORS 响应头**，无需源站支持 `OPTIONS`。

**语法：**
```text
cors <pattern>
```

**示例：**
```text
# 为指定 API 域名开启跨域支持
cors api.example.com

# 为所有流量开启跨域支持
cors *
```

---

## 5. 三个全局开关与上游代理

在「过滤」弹出层右栏顶部有三个实时开关：

1. **启用规则（Rules）**：临时关闭后，所有转发与改写规则暂停生效（但保留编辑器中的规则文本且不改写磁盘配置），顶栏「过滤」按钮变为警告色提醒。
2. **启用断点（Breaks）**：临时关闭后，仅跳过 `break` 与 `break-res` 断点拦截，其余规则（如 `map-local`、`map-remote`、`header`、`cors`、`replace`、`delay`）继续正常工作。
3. **隧道失败（Tunnel errors）**：默认关闭。源站 TLS 探测失败并退回透明隧道的连接中不含解密请求，默认不在列表中刷屏；开启后会将这类失败记录在列表中便于排查。

### 上游代理（Upstream Proxy）

在右栏底部的 **「上游代理」** 输入框（对应 `config.toml` 的 `upstream_proxy`）填写二级 HTTP/CONNECT 代理地址（例如公司网关或二级抓包工具），留空表示直连源站：

```text
127.0.0.1:7890
# 或带协议前缀
http://proxy.internal.corp:8080
```

---

## 6. 命名配置预设（Saved Setups）与导入导出

弹出层底栏可以将当前的 **「过滤表达式 + 直通名单 + 转发改写规则 + 上游代理 + 三个开关状态」** 打包保存为一套命名配置（最多保存 16 套，存储在 `config.toml` 的 `[[saved_profiles]]` 中）：

- **切换配置**：点击左下角下拉框选择任意预设，即可一键回填全部过滤与规则并立即生效。
- **保存配置**：输入配置名称后点击「保存」（若快速添加栏有已填写的草稿规则会自动并入）。同名配置（忽略大小写）会自动覆盖更新。
- **新建配置**：点击「新建配置」清空当前工作区的过滤、直通、规则与上游代理，恢复默认开关状态（不会删除已保存的配置预设）。
- **导出与导入 `.toml`**：
  - 点击 **「导出」** 将配置序列化为 `.toml` 文本并直接复制到剪贴板。
  - 点击 **「导入」** 展开大号多行输入框，粘贴 `.toml` 配置文本后点击 **「确认导入」** 即可导入并按名称合并预设（也支持直接将 `.toml` 文件拖拽到 `zniff` 窗口中导入）。

### 导出的配置预设 `.toml` 示例

```toml
[[profiles]]
name = "本地联调 API"
filter = "host:api.example.com|127.0.0.1 mute:/health"
bypass_hosts = "*.apple.com, *.icloud.com"
rules = """
# 本地联调转发与跨域
cors api.example.com
header api.example.com req set X-Env local-dev
map-remote api.example.com/v1 http://127.0.0.1:3000/v1
map-local api.example.com/v2/feature-flags ./mocks/flags.json
"""
upstream_proxy = ""
```
