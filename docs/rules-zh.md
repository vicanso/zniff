# zniff 规则与配置指南

[English](./rules.md) | 简体中文 | [文档索引](./README.md)

本文档系统介绍 **zniff** 的配置目录与开发隔离、流量过滤语法、TLS 直通名单（Bypass）、代理域名准入控制（`allow_hosts`）、17 类规则（转发、改写、断点、域名级控制、本地 Webhook 与 `.proto` Schema 解码）、动态模板变量与 `HMAC-SHA256` 签名、上游代理、外部根证书复用与本机设置，以及命名配置预设（Saved Setups）的导入导出。

---

## 目录

1. [配置文件与数据目录](#1-配置文件与数据目录)
2. [流量过滤语法（Traffic Filter）](#2-流量过滤语法traffic-filter)
3. [TLS 直通名单（Bypass List / 不解密）](#3-tls-直通名单bypass-list--不解密)
4. [转发、改写与扩展规则（17 类）](#4-转发改写与扩展规则17-类)
   - [4.1 通用语法与 `<pattern>` 匹配](#41-通用语法与-pattern-匹配)
   - [4.2 规则分类与速查总表](#42-规则分类与速查总表)
   - [4.3 终点规则（Terminal）](#43-终点规则terminal)
   - [4.4 叠加修饰规则（Modifier）](#44-叠加修饰规则modifier)
   - [4.5 域名级规则（TLS / DNS）](#45-域名级规则tls--dns)
   - [4.6 扩展与 Schema 规则（Webhook / Protobuf）](#46-扩展与-schema-规则webhook--protobuf)
   - [4.7 动态模板变量与签名（`{{...}}`）](#47-动态模板变量与签名)
   - [4.8 GraphQL 操作名匹配（`op=`）](#48-graphql-操作名匹配op)
   - [4.9 多组规则并行叠加（`[[rule_groups]]`）](#49-多组规则并行叠加rule_groups)
   - [4.10 从 Whistle / Charles 迁移导入](#410-从-whistle--charles-迁移导入)
5. [全局开关、上游代理与本机设置](#5-全局开关上游代理与本机设置)
6. [命名配置预设（Saved Setups）与导入导出](#6-命名配置预设saved-setups与导入导出)

---

## 1. 配置文件与数据目录

`zniff` 的持久化配置与根证书均保存在操作系统的用户数据目录下（由 [`ca::data_dir()`](../app/src/ca.rs) 解析）：

| 操作系统 | 数据目录路径 |
| :--- | :--- |
| **macOS** | `~/Library/Application Support/zniff` |
| **Windows** | `%APPDATA%\zniff`（例如 `C:\Users\<用户名>\AppData\Roaming\zniff`） |
| **Linux** | `$XDG_DATA_HOME/zniff`（未设置时为 `~/.local/share/zniff`） |

可通过 `--data-dir <目录>` 或环境变量 `ZNIFF_DATA_DIR` 覆盖数据目录（Docker 镜像默认 `ZNIFF_DATA_DIR=/data`）。

### 目录下的文件说明

| 文件名 | 用途 |
| :--- | :--- |
| `config.toml` | **正式版配置文件**（默认监听端口 `8877`）：保存监听设置、窗口位置/尺寸/显示器/模式、界面布局与列宽、主过滤词、直通名单、规则文本、上游代理与已保存预设（`saved_profiles`）。 |
| `proxy-snapshot.toml` | **正式版系统代理恢复快照**：开启系统代理前备份原系统网络设置；若进程异常退出，下次启动自动读取恢复。 |
| `config.dev.toml` | **开发模式配置文件**（`make dev`、Debug 构建或 `ZNIFF_DEV=1`，默认监听端口 `8878`）：与正式版完全隔离，避免开发调试覆盖日常配置。 |
| `proxy-snapshot.dev.toml` | **开发模式系统代理恢复快照**。 |
| `ca.crt` / `ca.key` | 本地生成的 `zniff CA` 根证书与私钥（正式版与开发模式共用；可通过 `external_ca` 切换复用本机已信任的 Whistle / mitmproxy / mkcert / Proxyman 外部根证书）。 |
| `certs/`（可选） | 按主机名（支持 `_wildcard.<domain>`）自动加载的自定义服务端证书目录。 |

> **临时大体积正文缓存**：抓包过程中超过 `16 KB` 的已完成非星标请求/响应正文会写入系统临时目录（如 `$TMPDIR/zniff-spill/`）的环形分片文件（单分片 64 MB，总上限 512 MB），退出或闲置 30 分钟后自动回收，不在配置目录中落盘。

---

## 2. 流量过滤语法（Traffic Filter）

`zniff` 提供两层协同过滤：
- **主过滤（持久化）**：位于顶栏 **「过滤」** 弹出层左侧（上方为「包含域名 / 包含 Path 前缀 / 排除域名或路径」三个结构化输入框与快捷筛选开关，下方为与之双向同步的完整过滤表达式），写入 `config.toml` 的 `filter` 并随配置预设保存。
- **列表二次过滤（会话临时）**：左侧域名/应用分面侧栏（`Split` 布局默认常驻，可收起）及列表顶部搜索框作用于当前主过滤结果内的临时下钻；搜索框非空时右侧出现 **「并入主过滤」** 按钮，可一键去重合并进主过滤并持久化。

过滤语法为**空格分隔的 `AND` 条件**，单个条件内支持使用 `|` 或 `,` 表示 **`OR`（或）**：

| 语法 / 关键词 | 用途说明 | 示例 |
| :--- | :--- | :--- |
| `host:<域名>` 或 `domain:<域名>` | 按主机名**精确匹配**（忽略大小写）、泛子域（`*.example.com`）或 `*` 通配匹配，支持多项 `OR` | `host:api.example.com\|*.cdn.com` |
| `path:<路径前缀>` | 按请求路径前缀匹配（可省略开头 `/`），支持多前缀 `OR` | `path:/api/v1\|/auth\|/graphql` |
| `ip:<IP>` / `client:<IP>` / `app:<进程名>` / `proc:<进程名>` | 按客户端来源 IP 或本地进程名匹配，支持多项 `OR` | `ip:127.0.0.1\|192.168.1.25`、`app:curl\|node` |
| `GET` / `POST` / `PUT` / `DELETE` / `CONNECT` 等，或 `method:<方法>` | 精确匹配 HTTP 方法（忽略大小写），支持 `\|` 多项 `OR` | `POST host:api.example.com`、`method:put\|patch` |
| `200` / `404` / `1xx`–`5xx`，或 `status:<状态码>` | 精确匹配状态码、状态码段或 `err` | `5xx`、`404`、`status:401\|403\|err` |
| `失败` / `fail` / `err` / `error` | 筛选发生错误（状态为 `ERR` 或状态码 `>= 400`）的请求 | `失败` |
| `type:<类型>` 或 `ct:<类型>` | 匹配响应 `Content-Type` 子串 | `type:json`、`ct:html`、`type:image` |
| `body:<关键词>` | 在解压后的请求正文、响应正文与 WebSocket/SSE 消息中搜索（支持检索落盘的大体积正文） | `body:access_token` |
| `header:<关键词>` | 在请求头与响应头的名称和值中搜索 | `header:authorization`、`header:x-trace-id` |
| `note:<关键词>` 或 `comment:<关键词>` | 按流量备注子串匹配（仅写 `note:` 匹配所有带备注的流量） | `note:复现`、`note:` |
| `repeat:` 或 `repeat:<id>` | 匹配批量重放生成的流量及其源请求；`repeat:<id>` 限定某个源请求 | `repeat:`、`repeat:42` |
| `op:<操作名>` / `gql:<操作名>` / `operation:<操作名>` | 匹配 GraphQL 操作名（显式 `operationName`、推断命名操作、`anonymous`、批量数组），支持 `*` 通配与 `\|` | `op:GetUser*\|Checkout` |
| `pin:` | 仅显示已加星标的请求 | `pin:` |
| `slow:` | 仅显示总耗时 `>= 1s` 的慢请求 | `slow:` |
| `mute:<词>` 或 `-<词>` | 排除主机或路径中包含该词的流量，支持 `\|` 分隔多项 | `mute:ads.example.com\|/health` 或 `-telemetry` |
| `-host:` / `-path:` / `-status:` / `-method:` / `-type:` / `-app:` / `-header:` / `-body:` / `-note:` / `-op:` / `-repeat:` / `-pin:` / `-slow:` | 以上任意结构化条件的反向（排除）写法 | `-host:*.cdn.com -status:3xx -pin:` |
| 普通裸词 | 在主机名（Host）、路径（Path）和流量备注中做不区分大小写的**子串匹配** | `login` |

> **提示**：`host:` / `domain:` 采用精确或通配符匹配（例如 `host:api.example.com` 不会误匹配 `sub.api.example.com`，需要子域请写 `*.example.com`）；若需主机名子串模糊查找，直接输入裸词即可。

**组合过滤示例：**

```text
host:api.example.com|*.staging.test path:/v1|/v2 POST type:json mute:/health
```

含义：匹配域名为 `api.example.com` 或 `*.staging.test`，且路径以 `/v1` 或 `/v2` 开头，方法为 `POST`，响应类型为 `json`，同时排除含 `/health` 的请求。

---

## 3. TLS 直通名单（Bypass List / 不解密）

位于顶栏 **「过滤」** 弹出层右栏的 **「直通名单 (不解密)」**（对应 `config.toml` 的 `bypass_hosts`）。
在收到客户端 TLS `ClientHello` 后按 SNI（无 SNI 时按 `CONNECT` 主机名）匹配：

- **空名单（默认）**：拦截并解密全部主机。
- **黑名单模式（默认）**：列出的域名跳过 MITM 解密，直接走透明 TCP 隧道（列表中记为 `直通名单，未解密`）。适用于启用了证书钉扎（Certificate Pinning）的服务。
- **白名单模式（`only:` 前缀）**：仅解密列出的域名，其余全部走透明 TCP 隧道直通。
- **分隔符与通配符**：支持空格、逗号 `,`、分号 `;`、竖线 `|` 分隔；支持精确域名、`*.example.com`（仅匹配子域，不含裸根域 `example.com`）与 `*`（全部主机）。
- **动态 `/proxy.pac` 自动配置**：抓包端口同步提供 `GET /proxy.pac`（与 `/zniff.pac`），结合 `allow_hosts`（未放行或严格全拒时返回 `DIRECT`）与 `bypass_hosts`（含黑名单与 `only:` 白名单模式）动态生成 `FindProxyForURL(url, host)` 脚本，非目标或回环地址自动直连（可在底栏 **「接入」** 弹窗一键复制 PAC URL）。

```text
# 示例 1：跳过 Apple 与指定钉扎域名的 TLS 解密
*.apple.com, *.icloud.com | pin.example.com

# 示例 2：白名单模式 —— 只解密 api.example.com 和 *.dev.internal，其余全部直通
only: api.example.com *.dev.internal
```

---

## 4. 转发、改写与扩展规则（17 类）

位于顶栏 **「过滤」** 弹出层右栏的 **「转发与改写规则」**（对应 `config.toml` 的 `rules`）。

- **实时语法校验与立即生效**：编辑器带行号并在右上角提示语法错误行号；修改后立即对后续新请求生效，无需重启代理。
- **单行规则辅助条**：编辑器下方可逐行一键 **注释/取消注释（`#`）**、**回填到快速添加表单修改** 或 **删除**。
- **16 项快速添加标签**：下方提供覆盖常用 16 类规则（除 `proto` 外）的表单化快速添加栏，并在支持动态变量的输入框下方提供 `{{ts_ms}}`、`{{uuid}}`、`{{hmac_sha256(...)}}` 等快捷插入胶囊。

### 4.1 通用语法与 `<pattern>` 匹配

- **一行一条规则**，`#` 开头或行内 `#` 之后为注释。
- 第二个词始终是 `<pattern>`（**内部不能有空格**），格式为 `<主机模式>[/<路径前缀>]`：
  1. **主机部分**：精确域名（`api.example.com`）、泛子域（`*.example.com`，不含裸根域）、全部主机（`*`），或多个主机用 `|` / `,` 连接（`api.example.com|*.staging.example.com`）。
  2. **路径前缀部分**（可选，省略或 `/` 匹配全部路径）：按路径段边界匹配（忽略 `?` 查询参数），`/api` 匹配 `/api`、`/api/`、`/api/v1`，**不**误匹配 `/apix`；多前缀在首个 `/` 后用 `|` / `,` 连接（`example.com/api|/v2|/graphql`）；任意主机写 `*/api|/v2`。
- 参数值或文件路径含空格、或以 `"` 开头时，用英文双引号 `"..."` 包裹（内部引号写 `\"`）。

### 4.2 规则分类与速查总表

| 类别 | 规则动词（别名） | 生效方式 | 支持 `op=` | 支持 `{{vars}}` | 核心用途 |
| :--- | :--- | :--- | :---: | :---: | :--- |
| **终点** | `map-local` | 命中首条 | ✅ | — | 本地文件 / 目录映射（支持 `fallback` 回源）或纯状态码空响应 |
| **终点** | `map-remote` | 命中首条 | ✅ | — | 转发到另一个 HTTP/HTTPS 目标地址（自动改写 `Host`） |
| **终点** | `break` | 命中首条 | ✅ | — | 请求发出前暂停，修改请求头/正文后放行或丢弃 |
| **终点** | `break-res`（`break-response`） | 命中首条 | ✅ | — | 收到源站响应后暂停，修改状态码/响应头/正文后放行 |
| **终点** | `block` | 命中首条 | ✅ | — | 直接返回拦截状态码（默认 `403`）或物理断连（`drop`） |
| **修饰** | `replace` | 顺序叠加 | ✅ | ✅ | 替换请求 URL/正文或响应正文（支持 `<= 1 MB` Chunked） |
| **修饰** | `json` | 顺序叠加 | ✅ | ✅ | 按 JSON Pointer 或点/下标路径改写/删除 JSON 字段（保序） |
| **修饰** | `header` | 顺序叠加 | ✅ | ✅ | 设置或删除请求头 / 响应头 |
| **修饰** | `query` | 顺序叠加 | ✅ | ✅ | 设置或删除 URL 查询参数 |
| **修饰** | `delay` | 顺序叠加 | ✅ | — | 注入请求延迟（累加总上限 `30s`） |
| **修饰** | `throttle` | 顺序叠加 | — | — | 按 4 KB 分片模拟弱网带宽限速（`2g`/`3g`/`4g` 或自定义速率） |
| **修饰** | `cors` | 顺序叠加 | — | — | 补齐跨域头，无终点规则时直接响应 `OPTIONS` 预检 `204` |
| **修饰** | `no-cache`（`nocache`） | 顺序叠加 | ✅ | — | 剥离条件缓存头并强制双端 `no-cache` / `no-store`，避免 `304` |
| **域名级** | `cert` / `client-cert`（`mtls`） | 按主机 | — | — | 挂载自定义服务端证书或上游 mTLS 客户端证书 |
| **域名级** | `resolve`（`dns` / `hosts`） | 按主机 | — | — | 自定义目标 IP/主机与端口解析（保留原 `Host` 与 TLS SNI） |
| **扩展** | `hook`（`webhook`） | 顺序叠加 | ✅ | — | 调用本机回环 HTTP 服务改写报文（`mutate`）或只读解密预览（`preview`） |
| **Schema** | `proto` | 只读解码 | — | — | 挂载 `.proto` 文件或目录，在详情页解码 Protobuf/gRPC 字段名 |

> **优先级说明**：**终点规则**按从上到下的顺序仅**第一条命中生效**；**叠加修饰规则**不占用终点名额，所有命中行按顺序累加生效（即使写在终点规则之后）。

---

### 4.3 终点规则（Terminal）

#### ① `map-local` — 本地文件 / 目录映射 / 纯状态码空响应

直接返回本地文件（或仅指定 `status=` 时返回空正文），**不连接源站**。响应 `Content-Type` 按扩展名自动推断。

当目标为**目录**时，`zniff` 去掉命中的路径前缀与 `?查询串`、百分号解码后在目录内按序解析：目录根或子目录优先匹配 `index.html`、`index.json`；无扩展名的路径依次尝试精确文件名与 `<路径>.json`（适配「生成本地 Mock…」导出的静态目录）。任何目录穿越路径都会被拒绝。

```text
map-local <pattern> [<本地文件或目录>] [status=<100..599>] [delay=<时长>] [fallback] [op=<GraphQL操作名>]
```

> 省略 `status=` 默认返回 `200`；省略文件路径时必须提供 `status=`；未加 `fallback` 时文件不存在返回 `502`，加上 `fallback` 后文件不存在则回源请求（备注记为 `map-local(fallback)`）。列表右键或详情「更多」点 **「本地映射」** 可将当前响应另存并自动追加规则。

```text
# 映射为本地 JSON（默认 200）
map-local api.example.com/v1/profile ./mocks/profile.json

# 含空格路径用双引号包裹
map-local api.example.com/v1/profile "/Users/alice/My Mocks/profile.json" status=200

# 纯状态码空响应，无需创建文件
map-local api.example.com/v1/maintenance status=503

# 模拟限流并延迟 1.5 秒
map-local api.example.com/v1/orders /tmp/rate_limit.json status=429 delay=1500ms

# 映射本地静态/Mock 目录，本地缺失的文件回源站
map-local www.example.com/static ./dist fallback
```

#### ② `map-remote` — 远程地址转发

将匹配请求转发到另一个目标地址（支持 HTTP/HTTPS 互转，自动重写 `Host` 头并保留后续子路径与查询字符串）。

```text
map-remote <pattern> <http://或https://目标地址> [op=<GraphQL操作名>]
```

路径拼接示例：`<pattern>` 为 `example.com/api`、目标为 `https://staging.internal:8443/v2` 时，请求 `https://example.com/api/users?id=1` 会转发至 `https://staging.internal:8443/v2/users?id=1`。

```text
map-remote api.example.com/api http://127.0.0.1:3000/api
map-remote test.example.com https://staging-api.example.com
```

#### ③ `break` — 请求断点

在请求发往源站前暂停拦截，顶栏出现黄色断点条：
- 点击 **「编辑请求」**（方法与 URL 只读展示）可修改 **请求头与请求 Body**，完成后点 **「放行」** 或按 `Cmd/Ctrl+Enter`。
- 点击 **「丢弃」** 直接关闭客户端连接或重置流（列表记为 `ERR · 断点丢弃`）；多条排队时可点 **「全部放行」**。
- 请求与响应断点共用最多 **8 条有界 FIFO 队列**：多条排队时断点条列出 `#id 请求/响应` 胶囊，点击任意胶囊将其提到队首并选中。队首无操作满 60 秒自动放行（编辑中自动延长超时）。

```text
break <pattern> [op=<GraphQL操作名>]
```

```text
break api.example.com/v1/checkout
break */auth/login
```

#### ④ `break-res`（或 `break-response`）— 响应断点

请求发往源站，收到完整响应（最多缓冲 `1 MB`）后、写回客户端前暂停拦截；可实时修改 **状态码、响应头与响应 Body** 后放行或丢弃。

```text
break-res <pattern> [op=<GraphQL操作名>]
```

```text
break-res api.example.com/v1/permissions
```

#### ⑤ `block` — 规则阻断与物理断开连接

在代理层直接拦截，不拨号源站：返回指定状态码（默认 `403`）与空正文，或使用 `drop` / `reset` / `abort` 立即切断底层连接/重置流（记为 `规则阻断 (连接已断开)`）。

```text
block <pattern> [status=<100..599>|drop|reset|abort] [op=<GraphQL操作名>]
```

```text
block telemetry.example.com status=403
block ads.example.com drop
```

---

### 4.4 叠加修饰规则（Modifier）

#### ⑥ `replace` — 文本查找替换

对 UTF-8 文本顺序执行查找替换（支持 `<= 1 MB` 带 `Content-Length` 或 `Transfer-Encoding: chunked` 报文；自动解压、改写并重算 `Content-Length`）：
- **`req`**：同时作用于请求目标（`/path?query`，改写结果须仍以 `/` 开头）与请求正文。
- **`res`**：作用于响应正文。
- `->` 右侧留空表示删除匹配原文；支持 `{{...}}` 动态变量与末尾 `op=`。

```text
replace <pattern> req|res <原文> -> [<替换为>] [op=<GraphQL操作名>]
```

```text
replace api.example.com/v1 res "\"env\":\"prod\"" -> "\"env\":\"debug\""
replace api.example.com/articles req draft=1 -> draft=0
replace api.example.com/api req ?debug=true ->
```

#### ⑦ `json` — JSON 字段精准改写 / 删除

在内存中改写 `<= 1 MB` 的 JSON 请求或响应正文（含 `chunked` 报文），严格保留原有字段键序。支持标准 JSON Pointer（`/a/b/0`）或点/数组下标路径（`a.b[0]`），值支持合法 JSON 字面量与 `{{...}}` 动态变量。

```text
json <pattern> req|res set <路径> <JSON值> [op=<GraphQL操作名>]
json <pattern> req|res del <路径> [op=<GraphQL操作名>]
```

```text
json api.example.com/v1/profile res set /data/is_vip true
json api.example.com/v1/items res set items[0].price 0
json api.example.com/events req del /metadata/device_fingerprint
```

#### ⑧ `header` — 请求头与响应头增删改

对请求头（`req`）或响应头（`res`）执行设置（`set` / `add`，不区分大小写覆盖同名头）或删除（`del` / `delete` / `rm`），值支持空格与 `{{...}}` 动态变量。

```text
header <pattern> req|res set <Header名> <Header值> [op=<GraphQL操作名>]
header <pattern> req|res del <Header名> [op=<GraphQL操作名>]
```

```text
header api.example.com req set X-Debug-Env staging-gray
header api.example.com/public req del Cookie
header *.example.com res del Content-Security-Policy
header *.example.com res set Cache-Control no-store
```

#### ⑨ `query` — URL 查询参数增删改

转发前对 URL 查询参数执行设置（`set` / `add`）或删除（`del` / `delete` / `rm`），自动处理分隔符与百分号编码，值支持 `{{...}}` 动态变量。

```text
query <pattern> set <参数名> <参数值> [op=<GraphQL操作名>]
query <pattern> del <参数名> [op=<GraphQL操作名>]
```

```text
query api.example.com/v1 set env staging
query *.example.com del utm_source
```

#### ⑩ `delay` — 延迟注入

在转发请求前等待指定时长；多条命中时累加，单次请求总延迟上限 **30 秒（`30s` / `30000ms`）**。

```text
delay <pattern> <时长(ms或s)> [op=<GraphQL操作名>]
```

```text
delay api.example.com/v1/feed 2000ms
delay api.example.com/v1/upload 5s
```

#### ⑪ `throttle` — 弱网带宽限速

按 4 KB 分片限制正文传输速率（仅限带宽，需模拟 RTT 请叠加 `delay`）：
- 预设（下行 / 上行）：`2g` / `slow-3g`（约 400 / 160 kbps）、`3g` / `fast-3g`（约 768 / 256 kbps）、`4g` / `slow-4g`（约 4 / 1 Mbps）。
- 单个速率表示上下行共用；`down=` / `dl=` 与 `up=` / `ul=` 分别设置。
- 单位（忽略大小写）：`kbps` / `kbit/s` / `kb` / 纯数字 = 千比特每秒；`mbps` / `mbit/s` / `mb` = 兆比特每秒；`kb/s` / `kbyte/s` = KiB/s；`mb/s` / `mbyte/s` = MiB/s；`b/s` / `bps` = 字节每秒。不支持单独的 `k` 后缀（如 `500k`）。

```text
throttle <pattern> <预设名|速率|down=<速率> [up=<速率>]>
```

```text
throttle api.example.com/mobile 3g
throttle cdn.example.com/video down=500kb/s
throttle api.example.com/upload down=2mbps up=512kbps
```

#### ⑫ `cors` — 跨域自动放行

为缺失对应响应头的匹配流量补齐：
- `Access-Control-Allow-Origin: *`
- `Access-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD`
- `Access-Control-Allow-Headers: *`
- `Access-Control-Max-Age: 86400`

当浏览器的 `OPTIONS` 预检请求命中 `cors` 且未命中任何终点规则时，直接返回 `204 No Content` + CORS 响应头，不拨号源站。

```text
cors <pattern>
```

```text
cors api.example.com
cors *
```

#### ⑬ `no-cache`（或 `nocache`）— 强制禁用缓存

彻底绕过客户端与源站 HTTP 缓存，避免 `304 Not Modified`：
- **请求侧**：剥离 `If-Modified-Since`、`If-None-Match`，并设置 `Cache-Control: no-cache, no-store, must-revalidate`、`Pragma: no-cache`。
- **响应侧**：剥离 `ETag`、`Last-Modified`，并设置 `Cache-Control: no-cache, no-store, must-revalidate`、`Pragma: no-cache`、`Expires: 0`。

```text
no-cache <pattern> [op=<GraphQL操作名>]
```

```text
no-cache static.example.com/assets
```

---

### 4.5 域名级规则（TLS / DNS）

> 以下规则仅按**主机名**匹配，不支持 `op=`；`resolve` 不允许在 `<pattern>` 中写路径或端口。

#### ⑭ `cert` / `client-cert`（`mtls`）— 自定义服务端证书与上游 mTLS

当 PEM 文件同时包含证书链与私钥时可省略 `<私钥路径>`：
- **`cert`**：向客户端出示自定义服务端证书，替代本地根 CA 动态签发的叶子证书。
- **`client-cert`**（别名 `mtls`）：源站要求双向 TLS 认证时向源站出示客户端证书（所用证书会展示在详情连接信息中）。

```text
cert        <host-pattern> <证书路径> [<私钥路径>]
client-cert <host-pattern> <证书路径> [<私钥路径>]
```

```text
cert *.internal.corp /etc/ssl/internal.crt /etc/ssl/internal.key
client-cert secure.bank.com ./certs/client.pem ./certs/client.key
mtls partner.example.com ./certs/partner-bundle.pem
```

#### ⑮ `resolve`（别名 `dns` / `hosts`）— 自定义主机 / IP 解析

类似本地 `/etc/hosts` 绑定，但支持泛域名与可选端口重写：底层 TCP 拨号指向 `<目标IP或主机>[:端口]`，同时**完整保留原始 HTTP `Host` 头、TLS SNI 与服务端证书主机名校验**。

```text
resolve <host-pattern> <目标IP或主机名[:端口]> [<端口>]
```

```text
# 将线上域名指向本地或灰度机 IP（保持原端口与原 HTTPS SNI）
resolve api.example.com 127.0.0.1

# 泛域名解析到内网测试机并改写目标端口为 8443
resolve *.staging.example.com 10.0.0.12:8443

# 支持 IPv6 目标地址
dns api.example.com [::1]:8080
```

---

### 4.6 扩展与 Schema 规则（Webhook / Protobuf）

#### ⑯ `hook`（别名 `webhook`）— 本机回环 HTTP 钩子扩展

通过任意语言（Python / Node / Go / Rust）编写的本机 HTTP 服务，对抓包流量执行自定义加解密、签名、脱敏或报文改写，无需在 `zniff` 中内嵌脚本虚拟机。

**安全护栏**：
- 仅允许指向本机回环地址（`http://127.0.0.1:...`、`http://localhost:...`、`http://[::1]:...`），防止配置误写导致流量外泄。
- 钩子请求直连，不进入 `zniff` 自身代理与会话列表；受限于 `timeout`（默认 `2000ms`，范围 `50ms..=30s`）与 `1 MB` 正文上限，超时或报错时记录备注并按原报文放行（fail-open）。

```text
hook <pattern> <http://127.0.0.1:port/endpoint> [req|res|both] [mode=mutate|preview] [timeout=2000ms] [op=<GraphQL操作名>]
```

两种工作模式与 JSON 协议：

1. **`mode=mutate`（默认）— 代理转发期原地改写**
   - `zniff` 向钩子 `POST` JSON：
     - 请求阶段：`{"id", "phase": "request", "method", "url", "host", "path", "headers": [["Name","Val"], ...], "body_base64", "body_utf8"}`
     - 响应阶段：`{"id", "phase": "response", "method", "url", "status", "headers": [["Name","Val"], ...], "body_base64", "body_utf8"}`
   - 钩子返回 `2xx` JSON（字段均可选，缺省表示不修改）：
     - `path`（仅请求阶段，须以 `/` 开头）、`status`（仅响应阶段，`100..=599`）、`headers`（`[["K","V"]]` 或 `{"K":"V"}`）、`body_base64` 或 `body_utf8`、`comment`（追加到流量备注）。

2. **`mode=preview` — 详情页只读解密预览（不改动转发字节）**
   - 代理转发完全零改动；打开详情页的「美化 / 树形」视图时，后台非阻塞调用本地钩子解密渲染并缓存结果，切回「原文」可对照原始密文。
   - `zniff` 发送：`{"id", "phase": "req"|"res", "mode": "preview", "method", "url", "status", "headers", "body_b64"}`
   - 钩子返回：`{"body_b64" | "body" (字符串或 JSON 对象/数组), "content_type"?, "note"? | "comment"?}`

```text
# 对支付接口的请求/响应做自定义 AES 加解密改写
hook api.example.com/v1/pay http://127.0.0.1:9001/mutate both timeout=3000ms

# 不改转发流量，仅在详情页自动解密查看密文响应
hook api.example.com/v1/cipher http://127.0.0.1:9001/decrypt res mode=preview
```

#### ⑰ `proto` — 零依赖 `.proto` Schema 挂载解码

挂载单个 `.proto` 文件或包含多个 `.proto` 文件的目录（纯 Rust 零外部依赖解析，**无需安装 `protoc`**），在详情页自动将 Protobuf / gRPC 二进制正文从通用 `field #N` Wire 视图升级为带真实字段名、枚举名称、嵌套消息、`oneof`、`map<K,V>` 的语义 JSON 与可折叠树形视图。

```text
proto <pattern> <proto文件或目录路径> [req=<pkg.ReqMessage>] [res=<pkg.ResMessage>]
```

- 对标准 gRPC `:path`（`/package.Service/Method`），省略 `req=` / `res=` 时会自动从 `service` 定义推断输入与输出消息类型。
- 对非 gRPC 路径的普通 Protobuf 接口，显式指定 `req=` 与 `res=` 即可解码。

```text
# 挂载整个 proto 目录，自动按 gRPC /pkg.Service/Method 路径推断消息类型
proto grpc.example.com ./protos

# 普通 HTTP Protobuf 接口显式指定请求与响应消息名
proto api.example.com/v1/order ./protos/order.proto req=shop.CreateOrderReq res=shop.CreateOrderRes
```

---

### 4.7 动态模板变量与签名（`{{...}}`）

`header`、`query`、`replace` 与 `json` 四类修饰规则支持在参数值中使用 `{{...}}` 动态模板表达式（代理转发时按当前请求实时求值，支持用 `+` 拼接字符串与变量；无法识别的 `{{...}}` 原样保留，不误伤模板文本）：

| 类别 | 可用变量 / 函数 | 说明 |
| :--- | :--- | :--- |
| **时间与随机** | `{{uuid}}`、`{{ts}}`、`{{ts_ms}}`、`{{iso_now}}`、`{{random_int(min,max)}}` | UUID v4、Unix 秒/毫秒时间戳、UTC ISO-8601 时间、区间随机整数 |
| **请求上下文** | `{{req.method}}`、`{{req.host}}`、`{{req.path}}`、`{{req.target}}`、`{{req.query.<key>}}`、`{{req.header.<name>}}`、`{{req.body}}` | 当前请求方法、主机、路径、含查询串目标、查询参数、请求头与请求正文 |
| **响应上下文** | `{{res.status}}`、`{{res.header.<name>}}`、`{{res.body}}` | 响应侧规则（`header ... res` / `replace ... res` / `json ... res`）可读取响应状态码、响应头与正文 |
| **摘要与编码** | `{{sha256(x)}}`、`{{sha256_b64(x)}}`、`{{hmac_sha256(secret, msg)}}`、`{{hmac_sha256_b64(secret, msg)}}`、`{{base64(x)}}`、`{{urlencode(x)}}` | 纯 Rust 实现的 SHA-256 / HMAC-SHA256（十六进制与 Base64）、Base64、URL 编码 |

> 当请求同时命中 `json`/`replace` 正文改写与引用 `req.body` 的 `header` 规则时，`zniff` 会在正文改写完成后自动重新求值请求头，确保签名始终基于最终发送的正文计算。

```text
header api.example.com/v1 req set X-Request-Id {{uuid}}
header api.example.com/v1 req set X-Timestamp {{ts_ms}}
header api.example.com/v1 req set X-Signature {{hmac_sha256("dev-secret", req.method + "\n" + req.path + "\n" + req.body)}}
```

---

### 4.8 GraphQL 操作名匹配（`op=`）

以下 **12 类规则**支持追加 `op=<操作名>`（或 `operation=<操作名>`）精准限定 GraphQL 生效范围：
`map-local`、`map-remote`、`break`、`break-res`、`block`、`replace`、`json`、`delay`、`header`、`query`、`no-cache`、`hook`。
（不支持 `op=`：`cors`、`throttle`、`cert`/`client-cert`、`resolve`、`proto`。）

- 自动提取显式 `operationName` 或从查询语句推断命名操作（含匿名操作 `anonymous` 与批量数组），明文 HTTP 与 HTTPS 均支持有界读取 `<= 1 MB` 请求体。
- 支持 `*` 通配符与 `|` 多操作名逻辑或。

```text
map-local api.example.com/graphql ./mocks/profile.json op=GetUserProfile
delay     api.example.com/graphql 3000ms op=Submit*
break     api.example.com/graphql op=Checkout|Payment
```

### 4.9 多组规则并行叠加（`[[rule_groups]]`）

除全局「当前规则」工作区（`rules`）外，`config.toml` 支持在 `[[rule_groups]]` 中维护最多 `32` 个可独立勾选/取消的命名规则组（同时同步 `--web` 操作台及 MCP `get_rules` / `set_rules`）：

- **生效顺序**：**当前规则（`rules`）优先**（便于右键快速填入 `break` / `map-local` 临时覆盖背景分组），随后按列表顺序拼接全部已启用的规则组；仍遵循同一套优先级模型（首个命中的终点规则生效，修饰规则顺序累加）。
- **界面操作**：在顶栏 **「过滤」** 弹出层规则编辑器上方点击分组胶囊切换编辑对象，点击左侧 `[✓/☐]` 复选框独立启用/停用该组规则，或点击 **`[+ 分组]`** 新建分组。

```toml
[[rule_groups]]
name = "本地联调环境"
enabled = true
rules = """
resolve api.example.com 127.0.0.1:8080
cors api.example.com
"""

[[rule_groups]]
name = "弱网测试"
enabled = false
rules = "throttle api.example.com 3g"
```

### 4.10 从 Whistle / Charles 迁移导入

- **Whistle `.txt` 规则**：支持直接将 `.txt` 规则文件拖入窗口，或在 **「过滤 → 导入」** 文本框中粘贴 Whistle 规则（自动识别 `host ip`、`http(s)://`、`file://`/`xfile://`、`reqHeaders://`/`resHeaders://`、`urlParams://`、`resDelay://`、`resCors://`、`statusCode://`、`disable://cache`、`resSpeed://`、`reqReplace://`/`resReplace://`、`ignore://` 与三反引号内联 `{block}`，无法直译条目以 `# [unmapped whistle] ...` 注释保留）。
- **Charles `.xml` 与 `.chlsj`**：支持导入 Charles 导出的 `.xml`（Map Local/Remote、Rewrite、Breakpoints、DNS Spoofing、No Caching、Throttling）与 `.chlsj` JSON 会话文件。
- **独立分组落盘**：转译后的规则会自动创建/更新为一个默认启用的命名规则组，同时将其中的直通域名并入 `bypass_hosts`。

---

## 5. 全局开关、上游代理与本机设置

### 5.1 三个全局开关与上游代理（随预设保存）

在顶栏 **「过滤」** 弹出层右栏顶部有三个实时开关，底部有上游代理输入框：

1. **启用规则（Rules）**：临时关闭后暂停所有转发与改写规则（不清空文本、不丢失配置），顶栏 **「过滤」** 按钮显示警告色提示。
2. **启用断点（Breaks）**：仅跳过 `break` 与 `break-res` 暂停，其余规则继续生效。
3. **隧道失败（Tunnel errors）**：默认关闭。开启后将源站 TLS 探测失败并退回透明隧道的连接记录在列表中便于排查。
4. **上游代理（`upstream_proxy`）**：支持二级 HTTP/CONNECT、`socks5://`（本地解析 DNS，默认端口 `1080`）或 `socks5h://`（代理端远程解析 DNS，默认端口 `1080`，支持 RFC 1929 用户名密码认证），留空直连源站：
   ```text
   127.0.0.1:7890
   http://proxy.internal.corp:8080
   http://alice:p%40ss@proxy.internal.corp:8080
   socks5://127.0.0.1:1080
   socks5h://user:p%40ss@127.0.0.1:1080
   ```

### 5.2 本机设置（不进已保存配置预设）

以下设置属于当前机器而非某套配置方案，修改后立即写入 `config.toml`，**不会**随 `saved_profiles` 切换或导出：

| 设置项 | 界面入口 | `config.toml` 键 | 说明 |
| :--- | :--- | :--- | :--- |
| **代理认证** | 底栏 **「安全」** 弹出层 | `proxy_auth` | `user:pass`。开启后抓包端口要求 HTTP Basic `Proxy-Authorization`（未通过回 `407`），转发前自动剥离该头；`/zniff`、`/zniff-ca.crt`、`/proxy.pac`、`/zniff.pac`、`/s/{uuid}`、`/r/{uuid}` 与回环 `/mcp` 豁免。可被 `--proxy-auth` / `ZNIFF_PROXY_AUTH` 覆盖。 |
| **允许代理的域名** | 底栏 **「安全」** 弹出层 | `allow_hosts` | 限制允许代理的目标域名。支持精确域名/IP/`[::1]`、`*.子域`、`*`（放行全部）、`-域名` 排除（优先于允许项）与 `#` 注释。优先级为 `--allow-hosts` > 非空 `ZNIFF_ALLOW_HOSTS` > `config.toml`（由 CLI/环境变量传入时界面与 `/api/hub` 只读锁定，写操作返回 `403`）。非严格模式留空放行全部；启用严格模式（`--strict-allow-hosts` / `ZNIFF_STRICT_ALLOW_HOSTS=1`，Docker 默认开启）且无正向规则时拒绝全部（返回 `403 Forbidden` 且不写入会话）。全链路作用于 `CONNECT`/SNI、HTTP/HTTPS/H2、`map-remote`、重放与 `/proxy.pac`，豁免本地路由（`/zniff`、`/zniff-ca.crt`、`/proxy.pac`、`/zniff.pac`、`/mcp`、`/s/`、`/relay`）。 |
| **复用外部根证书** | 顶栏 **更多 → 证书** / 底栏 **「接入」** | `external_ca` | 可选复用本机已信任外部根证书（`"whistle"`、`"mitmproxy"`、`"mkcert"`、`"proxyman"`）；留空使用内置 `zniff CA`。 |
| **中转地址** | 顶栏 **更多 → 中继设置** | `relay_url` | 用于实时分享的 `zniff --relay` 服务地址（空着只在本机分享）。 |
| **中转密钥** | 顶栏 **更多 → 中继设置** | `relay_token` | 上传到中继服务时使用的 Bearer 密钥。 |
| **接收中继** | 顶栏 **更多 → 中继设置** | `relay_accept` | 默认关闭；开启后允许接收他人推来的中继会话并在本机列表中查看。 |

---

## 6. 命名配置预设（Saved Setups）与导入导出

顶栏 **「过滤」** 弹出层底栏可将 **「主过滤表达式 + TLS 直通名单 + 规则 + 上游代理 + 三个开关状态」** 打包保存为命名配置预设（最多 16 套，存储在 `config.toml` 的 `[[saved_profiles]]` 中）：

- **切换与顶栏状态胶囊**：在底栏下拉框选择预设立即回填并生效（记在 `active_profile`）。顶栏会显示一体化 `[✓ <配置名/过滤条件>]` 状态胶囊：
  - 当主过滤非空时，点击胶囊可**一键暂停/恢复主过滤**（不影响规则与二次筛选）；
  - 当主过滤为空（例如纯规则配置）时，点击胶囊直接展开 **「过滤」** 弹出层，右侧摘要会预览规则条数（如 `规则×3: ...`）。
  - 取消选中当前配置可在预设下拉菜单顶部点击 **「取消选择当前配置」** 或再次点击已激活条目，不会删除该预设。
- **历史快照**：历史下拉自动保留最近 10 条配置快照（`profile_history`），随时回退。
- **保存与新建**：输入名称后点击 **「保存」**（快速添加栏中已填草稿会自动并入；同名忽略大小写覆盖；无改动时按钮置灰）。点击 **「新建配置」** 清空当前工作区并恢复默认开关，不删已保存条目。
- **导出与导入 `.toml`**：
  - 点击 **「导出」** 将配置序列化为 `.toml` 文本并直接复制到剪贴板。
  - 点击 **「导入」** 展开大号输入框粘贴 `.toml` 导入，或直接将 `.toml` 文件拖拽到 `zniff` 窗口上按名称合并导入。

```toml
[[profiles]]
name = "本地联调 API"
filter = "host:api.example.com|127.0.0.1 mute:/health"
bypass_hosts = "*.apple.com, *.icloud.com"
rules = """
# 本地联调转发、跨域与动态签名
cors api.example.com
header api.example.com req set X-Env local-dev
header api.example.com req set X-Timestamp {{ts_ms}}
map-remote api.example.com/v1 http://127.0.0.1:3000/v1
map-local api.example.com/v2/feature-flags ./mocks/flags.json
"""
upstream_proxy = ""
```
