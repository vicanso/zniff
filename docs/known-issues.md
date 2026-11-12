# zniff 已知限制

P0 到 P4 里点名的能力已经接上。下面是仍然接受的缺口，不挡当前使用；16 项产品交互与架构取舍的确认结论见 [docs/open-questions.md](open-questions.md)。

## 仍然不做

- HTTP/3、QUIC。UDP 不经过 HTTP 代理。
- Firefox 的证书库。Firefox 默认也不代理 localhost。验收浏览器是 Chrome、Safari、Edge。
- 透明代理、JSON 树、SQLite、cyberapi。桌面窗口仍是本机 nsh。Web 与桌面共用 `App`，编成 wasm 后由 `zniff --web` 提供。`make web` 提供 `app/.nsh/web` 里的共用界面，并在检测到 `app/src` 或 `app/Cargo.toml` 新于 `index.html` 时自动重跑 `make web-ui`；没有这份目录时才是应急小操作台。测试不加载这份目录。浏览器不抓包。抓包端口仍只在开始之后打开，而且只接受把该地址设为 HTTP 代理的客户端。网页不改系统代理。手机安装页、证书、分享和中继仍在抓包端口上。Web 列表通过 `/api/flows`（支持 `?filter=` 服务端全字段过滤）每次最多返回最近 2000 条（`MAX_LIST`），详情正文单次返回最多 256 KiB（`PREVIEW_CAP`）并通过 Base64 保持二进制无损往返，同时提供 `/api/export.har`、`/api/export.html` 与单条响应原文 `/api/flows/{id}/body`（支持从环形分片还原并遵守脱敏与严格遮罩配置，驱动 Wasm 端「另存响应…」）浏览器下载导出。浏览器端不能直接调用 `std::env::temp_dir()`、`std::process::id()`、`SystemTime::now()` 或 `Instant::now()`：wasm 上这几个函数会 panic（没有文件系统、进程号或时钟），墙钟统一封装在 `clock::now()`（桌面走 `SystemTime::now()`，wasm 走 `nsh_core::time::unix_ms()`）。大 body 的环形分片落盘在服务端/桌面进程里执行。本机已安装的 nsh 命令行若是 0.2，打不出这份 wasm，不要为了打包覆盖 `~/.nsh`。
- 会话重启清空。单条 body 解压后最多 1 MB（`BODY_CAP`），未星标 flow 完成后若请求或响应 body 超过 16 KB（`INLINE_BODY_CAP`）则顺序追加写入系统临时目录 `zniff-spill/` 下的环形分片文件（单分片上限 64 MB，最多保留 8 个分片共 512 MB，闲置 30 分钟自动删除过期分片，Unix 下打开后立即 `unlink` 匿名化以确保强杀进程由内核自动回收）并释放内存正文；内存会话最多 256 MB（`SESSION_CAP`），超出丢掉最旧的未星标 flow。星标最多 32 条（`PIN_CAP`），点星标时若 body 已落盘会提回内存且不参与淘汰，因此星标集合可以让内存超过 256 MB。线字节和 WebSocket 消息文本不计入这两个上限。主过滤词防抖写入 `config.toml` 的 `filter`，下次打开恢复。顶栏「过滤」最底一行从左到右是已保存配置下拉选择（`saved_profiles`，最多 16 条）、历史下拉（`profile_history`，保留最近 10 条配置快照）、配置名称、「导出」（复制 `.toml` 到剪贴板）、「导入」（粘贴或拖拽 `.toml`）、「新建配置」和「保存」。「保存」把当前过滤、直通、规则、上游和三个开关（启用规则、启用断点、隧道失败）写进 `saved_profiles`，点一条一起恢复。添加规则写进当前规则，底栏仍停在名称框里的那条。「新建配置」清空当前这套并回到默认开关，不删已保存的条目。过滤、规则、直通、上游都空着时不保存。名称对得上当前这条、内容也没变时「保存」是灰的，有改动才可以点。旧的 `saved_filters` 和 `saved_rules` 在还没有 `saved_profiles` 时并进来。停半秒才写入，避免每敲一个字都改文件。

## 平台

- 系统代理默认关闭。只有「更多」里的「系统代理」为开时，开始抓包才会改系统代理。启动不会单独打开它。上次关闭时若正在抓包，下次打开会自动开始；系统代理开关若是开的，这次开始仍会改系统代理。按「停止」后 `capturing` 写回 false。
- Linux 非 GNOME：开了「系统代理」也改不了，界面给出监听地址，需要自己指到 `127.0.0.1:端口`。监听仍会起来。
- Windows 只改当前用户的 WinINET，不改 WinHTTP。
- macOS `security add-trusted-cert` 可能要用户点确认。`security find-certificate` 只说明证书在登录钥匙串里，不保证 SSL 信任策略已经生效。
- 带密码的系统代理：`networksetup` 读不回密码，停止后可能恢复不完整。
- 证书状态检测与移除：macOS 用 `security find-certificate` 与 `security delete-certificate`，Windows 用 `certutil -user -verifystore / -delstore Root`，Linux 用 NSS `certutil -d sql:$HOME/.pki/nssdb -L / -D`（仅针对 Chromium 系使用的 `~/.pki/nssdb`，不改系统级 `/etc/ssl/certs`）。移除时同时清理旧名 `Snoop CA`。
- 窗口位置不记，窗口大小记住。`AppOptions` 提供初始 `size`；`window()` 返回表面尺寸，改变后停半秒写入 `config.toml` 的 `window_width` / `window_height`（默认 `1200×760`），下次启动由 `app/src/main.rs` 传入 `AppOptions::size` 恢复。

## 代理行为

- 没配上游时，源站连接直连，不走系统代理。`config.toml` 里配了 `upstream_proxy`（`[http://][user:pass@]host:port`）后，明文用绝对形式，TLS 先对上游发 `CONNECT`（带 `user:pass@` 时自动注入上游 `Proxy-Authorization: Basic`）；机器级 `proxy_auth`（或 `--proxy-auth` / `ZNIFF_PROXY_AUTH`）非空时，抓包端口对代理流量要求 HTTP Basic 认证（未通过回 `407` 且不写入会话，转发前自动剥离客户端 `Proxy-Authorization`；`/zniff`、`/zniff-ca.crt`、`/s/{uuid}`、`/r/{uuid}` 与回环 `/mcp` 豁免）。数据目录支持通过 `--data-dir` 或 `ZNIFF_DATA_DIR` 覆盖（Docker 镜像默认 `/data`）。
- 明文 HTTP 省略 `Connection` 时补 `Connection: keep-alive`。body 读完、双方都没要求关闭、状态不是 101 时才放回池。同一个键最多 4 条，键超过 32 就清空。HTTP/1 的 TLS 不进这个池。重放独立建连且不进池：HTTP/1.1 强制 `Connection: close`，源流为 `HTTP/2` 或独立构造 HTTPS 请求时宣告 `["h2", "http/1.1"]` ALPN 并按协商结果使用单次独立 HTTP/2 或 HTTP/1.1 连接发送。
- HTTP/2 在一条客户端连接上复用源站连接，另外缓存最多 16 条发送端。空闲超过 45 秒就丢掉这条缓存，连接驱动结束时也删掉这一代。这条浏览器连接发现连接已关、空闲过久或发送失败时，会换一条新的源站连接；请求还没送出时再送一次。已经写进连接的请求不重试。
- 源站 TLS 探测失败后会关掉那条探测连接，另开一条 TCP，再把缓冲的 ClientHello 放进去。探测失败的同一条 TCP 不能复用。
- 源站没选 ALPN 时按 HTTP/1.1 记。
- ServerHello 之前失败改成 TCP 隧道，只记目标和原因。源站 TLS 探测失败也改隧道转发，默认不在列表里记一行：这条连接没有解密出的 HTTP。配置弹出层里的「隧道失败」打开后才记，并写入 `config.toml` 的 `show_tunnel_failures`。连接源站超时、不是 TLS、直通名单仍记一行。ServerHello 之后的解析失败只记错误，不能退回隧道。
- 图片预览按原图像素画，小图不放大。宽或高超过 200 时等比缩小。
- ServerHello 之后客户端断开，界面记成「证书被拒绝」，并附一句猜测。收到 `unknown_ca` 一类告警时，猜客户端不信任 zniff CA。收到握手失败告警，或连接被重置且主机像 Google、Apple、Microsoft 这类常钉扎的名字时，猜是证书钉扎，建议加入直通名单。其余重置先让用户核对 CA 是否已信任。这仍不是钉扎的专用检测。
- 转发到源站失败时，错误仍以步骤名开头（例如「HTTP/2 上游」），后面附一句猜测。发送端报连接已取消、耗时接近 0 时，猜的是复用的 HTTP/2 连接已经关掉，下一次会重新连接。重置、拒绝连接、对端提前关闭、源站要求 HTTP/1.1、主机名解析失败和超时会换成对应的那一句。看不出具体原因时，只说明这一步失败了。这是按错误种类推断的，不是源站给出的原因。已经记下的旧行仍是当时写下的短句。
- HTTP/2 传输层只表示两边用源站选定的协议，不解析底层 HTTP/2 二进制帧、HPACK 表或 trailer；但对承载的 Protobuf / gRPC（`application/grpc*` 5 字节长度前缀帧）与 MessagePack 正文，详情「美化」模式支持无 Schema Wire 解码与 JSON 预览。
- 界面里非 UTF-8 body 原文只显示前 4 KB 十六进制；UTF-8 正文（含美化结果）按 256 KB 分页展示，标题栏有「往前」「往后」按字符边界继续翻阅至 1 MB 记录上限，并支持正文内关键字搜索（跨流保留搜索词，多页正文检索时标题栏标注「当前页命中」）及 HTML/Markdown 渲染预览。记录本身仍保留最多 1 MB。复制用的是记录原文。左栏按到达时间排列，最新在最下面；离底部不超过一行时跟着新流量，再往上翻就停在当前内容。响应 body 在内层已经 `is_end_stream()`，或 `RecordingBody` 被丢弃时记完成。空响应（例如 `304`）和最后一帧不会停在「进行中」。超过 1 秒的时间列用警告色。主机列保持正常颜色。没有状态码的失败行，路径列用错误色。
- WebSocket 从旁路解析帧并立刻转发。转发握手时去掉 `Sec-WebSocket-Extensions`，避免谈成 `permessage-deflate`。非法帧或仍然带扩展位（`RSV1 != 0`）只停止解析后续帧，不切断对拷。消息最多 200 条或 256 KB 文本。`Content-Type` 含 `event-stream` 且没有 WebSocket 消息时，按 SSE 事件列出。消息卡片支持每帧右侧一键复制及标题栏「复制全部」导出当前筛选后的消息帧。
- 规则里 `map-local`、`map-remote`、`break`、`break-res`、`block` 是终点，按写下来的顺序第一条命中；`delay`、`throttle`、`cert`、`client-cert`（或 `mtls`）、`header`、`query`、`cors`、`no-cache`、`replace`、`json` 不占这条名额，匹配到的都会叠加上去，包括写在终点后面的行，且除 `cors`、`throttle`、`cert` 外的 11 种规则均支持可选 `op=<GraphQL操作名>` 条件（支持 `*` 通配与 `|` OR；明文 HTTP 与 HTTPS 均支持有界读取 `<= 1 MB` 请求体提取 GraphQL 操作名）。规则的域名段和 Path 前缀段均支持用 `|` 或 `,` 表示多个 OR（例如 `replace a.com|*.b.com/api|/v2 res ...`）。文件路径、匹配原文、替换目标、头值或查询参数含空格时用双引号 `"..."` 包裹。`replace 主机/路径 res 原文 -> 替换成` 改响应正文，`req` 改请求目标（path 和 query 连在一起）和请求正文。目标改完仍须以 `/` 开头，否则这一处不改；没有 body 也会改目标。`replace` 与 `json` 的正文改写支持带 `Content-Length <= 1 MB` 的报文，且当明确命中至少一条 `replace` 或 `json` 规则时也会对无 `Content-Length` 的 `chunked` 流式报文按需有界缓冲最多 1 MB：先按 `Content-Encoding` 解开，改写后去掉压缩头并重算 `Content-Length` 发明文；未命中改写规则的 chunked 流量仍零缓冲流式直通，对不上、不是 UTF-8（或非 JSON）或超过 1 MB 时照原样转发（超限跳过在备注记录原因）。同一条又命中响应断点时，这一轮不替换，断点仍看源站正文。顶栏「过滤」弹出层右栏里的规则框是带行号的编辑器，右上角会提示语法有误的行号。下面可以通过 14 项分段标签（替换、转发、文件[本地/回源]、延迟、断点、响应断点、改头、改参、跨域、限速、证书[MITM/mTLS]、JSON、禁缓存、阻断[状态码/断开]）快速追加一行，匹配行支持分别填写多域名 OR、多 Path 前缀 OR 以及可选 `GraphQL op`（在全部 11 种支持 `op=` 的规则标签上统一展示）。域名和 Path 都可以留空，留空写成 `*` 匹配全部。行右键「填入断点」写入 `break 主机路径`，查询串不放进去。改已有的哪一行仍没有单独的表单编辑器。`delay` 最长 30 秒。`map-local` 可写 `status=429`（省略时是 200，也可省略文件路径仅返回状态码与空正文），目标为本地目录时按匹配前缀后的相对路径解析静态资源并支持 `fallback` 回源。`header` 用 `req` 或 `res` 再写 `set` 或 `del`；`query` 支持 `set` 或 `del` URL 查询参数；`cors` 给响应补跨域头，`OPTIONS` 且没有终点规则时直接回 204，不拨源站。明文在拨号前应用，HTTPS 在源站连接打开后应用，WebSocket 和 `CONNECT` 跳过。Map Remote 换了主机就另开连接。请求断点和响应断点进入同一个最多 8 条的有界 FIFO 队列（`BREAK_QUEUE_CAP = 8`），队满时自动放行队首最旧的一条；队首默认等 60 秒（在编辑框内编辑时延长至 5 分钟），超时当放行，排在后面的不会先超时；点击断点条上的 `#id 请求/响应` 胶囊可将其提到队首并选中；点击断点「丢弃」直接关闭客户端连接或重置流（无状态码，记 `ERR · 断点丢弃`）。响应断点先缓冲 body，最多 1 MB；已经写明的 `Content-Length` 超过 1 MB 时不暂停，记「响应断点跳过」后照常转发；分块或未知长度读过 1 MB 时，这条响应记失败。请求断点放行时可改请求头和请求 body（Method 与 URL 只读展示）；响应断点放行时可改状态码、响应头和响应 body。重放不套规则和断点。顶栏「过滤」弹出层右栏顶部有「启用规则」和「启用断点」，默认都开，随配置写入 `config.toml` 与 `SavedProfile`。关掉规则后，Map、替换、延迟、头、CORS 和断点都不生效。只关掉断点时，其它规则仍生效，`break` 和 `break-res` 不再暂停。已经停住的那一条仍要等「放行」、丢弃或超时。两个开关有一个关掉时，顶栏「过滤」按钮用警告色。
- 直通名单在 ClientHello 之后看 SNI，没有 SNI 就用 CONNECT 主机。支持空格、`,`、`;` 或 `|` 分隔多个域名 OR。`*.example.com` 匹配子域，不匹配顶点。`only:` 是白名单。`*` 全部直通。空名单全部解密。
- `GET` 或 `HEAD` 的路径去掉 query 后是 `/zniff-ca.crt` 时直接返回根证书 PEM。直接访问 `/`，或路径是 `/zniff`，返回手机安装页。`Accept-Language` 首段不是 `zh` 时安装页是英文，空的或 `zh` 仍是中文。代理请求里的 `http://其他主机/` 仍转发出去。
- 监听默认 `127.0.0.1:8877`。端口和局域网开关记在 `config.toml` 的 `listen_port`、`listen_all`，`0` 拒绝，绑不上就报错，不悄悄换端口。端口已被占用时说明多半是另一个 zniff 还在监听，并提示关掉它或改底栏端口。`listen_all` 打开后绑 `0.0.0.0`。监听地址是 `0.0.0.0` 之后，底栏「扫码」点开才显示安装页二维码。默认地址是访问公网的那张 IPv4；隧道和 `198.18.0.0/15` 让位给物理网卡。多张网卡可改选。文件导入导出与响应另存使用 `nsh 0.5` 的 `pick_paths` / `pick_save_path` 系统文件对话框。图片预览支持 png、jpeg、webp、gif、ico（ico 自动解包或转为 PNG 缓存供跨平台渲染），且响应已结束、非空、未截断。SVG 不预览。

## 已接上，不再当作待办

这些有库测试：

- 清空会话。顶栏有「清空」按钮，左侧「统计」按钮内置会话条数角标（没有过滤词时只显示总数，有过滤词时显示「筛选数 / 总数」）；「更多」首行提供导出 `[HAR][HTML]`，下方提供「清空未星标」。清的是整个会话。清列表、字节和丢弃计数，不清下一条 id。切换选中行或点「清空」会收起「编辑重发」并清掉编辑框。
- 复制 URL、cURL、`fetch`、Python `requests`、Rust (`reqwest`)、Go (`net/http`)、请求 body、响应 body，以及响应 JSON 的浅层 TypeScript `interface`（`TS 类型`）与 Rust `serde` `struct`（`Rust 结构体`）。cURL 跳过 `Host` 和 `Content-Length`。非 UTF-8 复制十六进制。二进制请求 body 不写进 cURL。
- 记住监听端口。摘要里有开始时间、TTFB、总耗时、请求和响应线字节。时区用启动时的 `date +%z`，否则 UTC。HAR 导入按 `timings.wait`、`time` 和 `bodySize` 回填 `response_at`、`finished_at`、`request_bytes`、`response_bytes`，并支持读写 `_webSocketMessages` 无损往返 WebSocket/SSE 消息帧。`bodySize` 缺失或为负时用已还原的 body 长度。
- JSON、表单、XML、HTML 默认美化，可切「原文」。HTML 与 Markdown 响应/请求各自可在卡片标题栏独立切换「预览」（基于 `TextView`）与源码视图，切流或清空时自动复位。`config.toml` 的 `default_pretty` 缺省为开；「更多」里「默认美化」关掉后下次启动用原文，并马上按新模式重画当前选中的详情。卡片上的「美化 / 原文」仍只改当前窗口。复制和另存仍用记录原文。查询参数单独成表。美化时 `x-www-form-urlencoded` 的请求 body 也按键值行画，值下面带 JWT、时间戳和 Base64 提示；原文仍是整段。另存响应、导出 HAR 与导出 HTML 均通过系统原生保存对话框选择路径（通过 `Owner::detached()` 启动异步文件选择任务，避免菜单关闭销毁 `Owner` 取消任务），状态栏给出完整路径并复制到剪贴板。
- 过滤是 AND 词，同一个词内支持 `|` 或 `,` 表示 OR：`host:` / `domain:`（支持 `*.` 通配符与多域名 OR）、`path:`（支持多 Path 前缀 OR）、`ip:` / `client:` / `app:` / `proc:`、方法精确、状态精确或 `1xx`–`5xx`、`失败`、`type:` / `ct:`、`body:`、`header:`、`note:` / `comment:`、`repeat:`、`op:` / `gql:` / `operation:`、`pin:`、`slow:`、`mute:`、`-词` 及带负向前缀的结构化字段词（`-host:` / `-domain:`、`-path:`、`-app:` / `-proc:` / `-ip:` / `-client:`、`-status:`、`-method:`、`-type:` / `-ct:`、`-header:`、`-body:`、`-note:` / `-comment:`、`-op:` / `-gql:` / `-operation:` 等），其余匹配主机、路径或备注。`body:` 搜请求 body、响应 body 和 WebSocket/SSE 消息，`header:` 搜头名和头值。`pin:` 只留星标。`slow:` 只留从开始到结束不少于 1 秒的流量；还没结束的用现在的时间，和列表警告色是同一条规则。`mute:` 和 `-词` 排除主机或路径里带这个词的行，`-字段:值` 排除匹配该结构化条件的行。顶栏「过滤」双栏面板左侧有域名 OR、Path 前缀 OR、排除域名/路径 OR 三个结构化输入框（与完整过滤表达式双向同步），以及 JSON、报错、星标、慢请求四个快捷筛选开关。点一下追加整词 `type:json`、`失败`、`pin:`、`slow:`，再点去掉。已经写了 `ct:json` 或 `fail` 时，对应开关也算打开，再点会把这些别名一起去掉。左栏顶部的搜索框、分面导航胶囊以及列表右键/详情「更多」里的「只看」「屏蔽」作用于当前会话的二次筛选（`list_filter`），非空时搜索框右侧提供「并入主过滤」一键去重并入 `profiles.filter` 并保存，顶栏状态摘要合并反映主过滤与二次筛选条件。复制、另存、星标、基准、备注、重放、批量重放、编辑重发、填入断点、本地映射和美化也在这个菜单里。
- 「编辑重发」可改方法、URL、头和 body（未选中流量时点「构造请求」直接新建请求并发送；左栏行右键菜单同样提供「编辑重发」/「编辑请求」/「编辑响应」）。编辑区内支持 `Cmd/Ctrl+Enter` 一键发送/放行，URL 单行框支持按 `Enter` 发送（输入以 `curl` 开头时仅解析填充不误发）。URL 框以 `curl` 开头时，支持行尾 `\` 换行续行与 `$'...'` ANSI-C 转义，并按 `-X`、`-I` / `--head`、`-H`、`--url`、`--json`、`-d` / `--data*`（多段以 `&` 拼接）、`-G` / `--get`（将 data 拼入 URL query）、`-u`、`-A`、`-e`、`-b` 填进这些框；带 body 又没写方法时当成 `POST`。发送时按新 body 重算 `Content-Length`。URL 改了就丢掉旧的 `Host`。停在请求断点上点「编辑」显示「编辑请求」与「放行」（Method 与 URL 只读展示，改请求头和请求 body 后放行）；停在响应断点上点「编辑」显示「编辑响应」与「放行」，改状态码（100–599）、响应头和响应 body。
- 复制菜单包含 URL、cURL、`fetch`、Python `requests`、Rust (`reqwest`)、Go (`net/http`)、`TS 类型`（浅层 TypeScript `interface`）与 `Rust 结构体`（`serde` `struct`）。桌面端「更多」里「复制代理环境」复制 `http_proxy`、`https_proxy`、`all_proxy`、`SSL_CERT_FILE`、`NODE_EXTRA_CA_CERTS`、`REQUESTS_CA_BUNDLE`、`CURL_CA_BUNDLE`、`GIT_SSL_CAINFO`、`CARGO_HTTP_CAINFO`、`DENO_CERT`、`JAVA_TOOL_OPTIONS`，证书路径指向数据目录的 `ca.crt`，端口用当前记住的监听端口。请求头和查询值里能认出的 JWT、10 位或 13 位时间戳、可打印 Base64 会附在原值下面。两条流量可以用「基准」做行级对比。设为基准的那一行，方法列最前有一条红竖线，和选中时的蓝线同一位置；这一行如果正在被选中，仍显示蓝线。对比卡片顶部用同一红色标「基准」、用绿色标「当前」：红的是基准，绿的是当前选中。再点这条的「基准」会取消，菜单写成「取消基准」，红线和对比卡片一起去掉。换一条再点「基准」会改成新的。超过 1 秒的时间列用警告色。星标在方法徽章前有蓝点，藏起主机列也看得到。
- 直通名单、上游绝对形式、明文连接池、WebSocket 文本帧、握手去掉 `Sec-WebSocket-Extensions`、Map Local、断点丢弃、HTTPS 上的 Map Local / Map Remote / 断点放行、HTTP/2 发送端缓存命中、空闲过期后重连、同一条客户端连接上换源站连接、HTTP/2 Map Local、HTTP/2 断点丢弃、重放改方法 URL 头和 body、`/zniff-ca.crt`、安装页 `/zniff` 有代理往返测试。HAR、规则语法、SSE、美化、`body:` / `header:`、`status=` / `delay` / `header` / `cors`、响应断点改写、星标不淘汰、环境变量、`fetch` / Python / TypeScript、cURL 解析、JWT 提示和行级 diff 有单元测试。

## 界面

左栏默认宽 660，可拖到 280–1100。方法、状态、路径、间隔、耗时、主机、协议、大小、URL、头和 body 用等宽。按钮和句子用界面字体。英文界面的按钮和说明不改成等宽。

- 左栏顶部提供二次过滤搜索框（在当前主过滤结果中再次筛选 `list_filter`，有内容时带「并入主过滤」合并按钮与 `×` 一键清空按钮）、右侧可折叠的「域名与应用导航」分面抽屉开关及列显隐图标。完整的流量过滤与规则代理配置收纳在顶栏「过滤」双栏面板里，按钮右侧显示当前生效的规则条数。
- `↑` / `↓` 在当前筛选后的列表里换选中行。没有选中时 `↓` 选第一条，`↑` 选最后一条，到头就停住。选中的 id 已经不在筛选结果里时，按同样的规则重新选。换行会收起「编辑重发」。过滤框或其它输入框有焦点时，方向键仍移动光标。`Cmd/Ctrl+K` 清空整个会话，和「更多」里的「清空」一样。`Cmd/Ctrl+F` 聚焦过滤框。`Cmd/Ctrl+R` 重放当前选中行，没有选中时状态栏提示需要选择。`make dev` 的热重载可能吃掉 `Cmd+R`。方向键不滚动列表。
- 列表共 10 列，默认开启 7 列：序号、方法、路径、状态、间隔、耗时、大小（`show_id`、`show_method`、`show_path`、`show_status`、`show_gap`、`show_elapsed`、`show_size` 默认开）；协议、主机、来源 IP 默认关闭（`show_protocol`、`show_host`、`show_client_ip` 默认关，旧配置文件没有这些键时保持默认）。可见的序号、方法和路径固定在左侧，横向滚动时不离开；其余列在滚动区横向滚走。支持直接拖拽表头或在列菜单内点击上移/下移调整列顺序（固定区与滚动区各自内部调序，记入 `column_order`，菜单内可一键恢复默认列序），并支持点击表头按升序 `↑` / 降序 `↓` / 默认顺序三态切换排序。每一列的右缘可以拖，宽度记在 `column_widths`，下次打开还在。路径默认 220，主机默认 120。列图标和表头右键按表格列顺序一对一开关各列并写进 `config.toml`。间隔是距当前列表里上一条的时间，首条 `0 ms`（`45 ms`、`1.24 s`，再长用分和小时）。过滤之后按看得见的行重算，被滤掉的行不计入。悬停显示本地 `HH:MM:SS.mmm`。详情摘要用同一绝对时间。耗时列是这条请求从开始到收完响应，和详情里的「耗时」同一段，还没结束是 `—`，不少于 1 秒用警告色。藏起一列后，它不占宽。显隐不靠表格自己的开关，只把要显示的列交给它。列设置菜单底部可切换路径列显示「仅路径」或「完整 URL」（`show_full_url`，默认 false，开启时列标题变为 `URL` 且显示 `flow.url`），以及路径和主机超出时的省略位置（`ellipsize`：`head` 开头、`middle` 中间、`tail` 结尾，默认 `middle`），悬停显示全文。协议列用和摘要徽章相同的名字，大小列是响应的线字节。详情摘要在协议徽章旁边显示这条流量实际命中的规则名，例如 `delay replace break`。没有命中就不显示。左栏与右栏之间可拖，松开后把宽度记到 `list_width`，限制在 280 到 1100（同时随当前窗口宽度实时约束上限为 `window_w - 340`），默认 660。顶栏「过滤」弹出层宽度按 `window_w - list_px * 0.7 - 16` 在 680–1360 自适应，规则编辑器高度按双栏剩余高度预算在 84–460 自适应。分割条宽 6，悬停时中间的线变宽并改成强调色。
- 没有状态码的失败行，列表状态是 `ERR`，路径用错误色显示记录里的原因。转发失败的原因是步骤名加一句猜测。窗口会翻译一批固定原因，原文仍是写入时的语言。进行中、还没有错误的行状态是 `—`。过滤仍用记录里的真实路径。详情摘要在没有状态码且有错误时徽章是 `ERR`，否则仍是「等待响应」。
- 详情在摘要之后是响应 body、响应头、请求 body（有正文、表单或说明才画）、查询参数、请求头，然后是行级对比和消息。查询、对比、消息、两张 body 和两张头卡片可以点标题折叠，摘要不折叠。这些标题都在卡片顶栏里，左内边距相同。折叠只在这次窗口里记住，不写入配置，换选中行也不重置。请求 body 为空且没有截断或解压说明时不画那张卡片。响应 body 没有正文、没有说明、美化模式下也没有图片时同样不画。图片在美化时按比例画进响应 body 卡片的预览框，不裁切铺满，并写上字节数；这时去掉「十六进制只显示前 N 字节」这句，解压失败等其它说明仍留下。原文显示十六进制，默认每 16 字节一行，这块区域摆开后宽度够 32 字节时改成一行 32 字节，两组 16 字节之间多一个空格；行首是 `0001` 这种四位行号；复制仍是不带行号、每 16 字节一行的记录。并保留那句截断说明。JSON（含 GraphQL 分块美化）、HTML、XML、Protobuf/gRPC Wire 解码、MessagePack 转 JSON，以及不少于 80 字节的多行文本，画在只读 `editor` 里，带行号和折叠；HTML 与 Markdown 在请求和响应卡片上各有独立的「预览」开关。请求和响应 body 按行数撑开，一行高 18，最多 15 行，再多在卡片里滚动。短文本和十六进制同样不超过 15 行。没有 `Content-Type` 或标成 `text/plain`、但正文以 `{` 或 `[` 开头的，也按 JSON 高亮。短文本和十六进制仍是普通等宽文本。「编辑重发」的头和 body 用可编辑的 `editor`。有基准时，对比卡片用 `diff_view` 的上下对照，两边是 URL、请求头、请求 body、响应头、响应 body。两张 body 卡片的标题上都有「美化 / 原文」，点标题不会切到美化。菜单里仍可切换。美化时 `Cookie` 按 `;` 拆成一行一个键，`Set-Cookie` 保持单行不拆分；原文和复制仍是整段。美化时请求头和响应头按名字忽略大小写升序，同名保持原来的先后和大小写；原文、cURL 和复制不排序。
- HAR 导出与「导出 HTML」通过系统保存对话框选择保存路径；导出/分享脱敏（`redact_export`）覆盖敏感头、Cookie、URL query、JSON、表单、`multipart/form-data` 字段、SSE/WebSocket JSON 帧，并支持可选「严格遮罩」（`redact_strict`，对无法结构化脱敏的二进制或未知格式正文直接替换为遮罩占位符）。过滤框有内容时只导出当前匹配的流量，「更多」里「导出」右侧是将写出的条数；过滤框空着仍导出整个会话，这时不显示条数。「分享 15 分钟 / 1 小时 / 8 小时」生成 UUID，地址是 `http://{地址}:{端口}/s/{uuid}`，页面每 2 秒按分享当时过滤框里的条件看会话，之后改过滤框不会改这条链接。过滤框空着就是整个会话。同时只有一条链接，停抓包或「停止分享」后失效。没开局域网时只在本机。配置了中转地址后，剪贴板链接指向该地址的 `/s/{uuid}`，本机大约每秒推送同一份会话；地址空着仍是本机链接。对方若开了「接收中继」，这些流量同时进那台 zniff 的普通列表，只能查看、过滤和复制。`zniff --relay` 仍然只提供分享页。浏览器打开的分享地址是分享页。`zniff --web` 是另一个端口上的操作台，不抓包，抓包端口要等开始才打开。分享请求不记入会话。实时分享与「导出 HTML」共用同一套只读查看器；实时分享 `flows.json` 保留最多 400 条较新的记录（超过 16 KB 的落盘正文在点选详情或基准对比时通过 `/s/{uuid}/flows/{id}.json` 按需加载，消息最多 200 条）。导入 HAR 在顶栏「更多」里点击弹出系统文件选择框，把 `.har` 拖到窗口上也会导入。

## 后续吸引力增强方向（这一轮跳过）

上面第 1 到第 4 组里，环境变量、响应断点、状态码和延迟、头改写、CORS、行级对比、只看 / 屏蔽 / 星标、慢请求在时间列着色、JWT 与编码提示、`fetch` / Python / TypeScript、粘贴 cURL，都已经接上。下面这些不做：

- **网卡级随机丢包模拟**。应用层带宽限流已通过 `throttle` 规则支持（支持 `2g`/`3g`/`4g` 预设及 `down=`/`up=` 速率限制），但不做 TUN/TAP 网卡层的随机丢包与乱序重传模拟。
- **JSON 树对比**。对比是 URL、请求头、请求 body、响应头、响应 body 的行级差异，每边最多 200 行。不做可折叠的 JSON 树。
- **超过 1 MB 的响应改写**。响应断点的缓冲上限就是单条 body 上限。更大的响应只转发，不在界面里改。
- **完整 cURL 语法**。已支持行尾 `\` 换行续行、`$''` ANSI-C 转义以及 `--json`、`-G` / `--get`、`-I` / `--head` 与多段 `-d` 拼接，但不解析涉及本地文件读取的 `-F` / `--form`。认不出的旗标忽略。
- **非 JSON 的 TypeScript**。响应不是 JSON 对象或数组时，「类型」提示不可用，不猜表单或 XML。
- **正则表达式（Regex）过滤与正则替换**。当前多域名 OR（`host:a.com|*.b.com`）与多 Path 前缀 OR（`path:/v1|/auth`）已覆盖绝大多数多服务联调场景；引入正则会带来转义负担与全量流量匹配开销，暂不引入。
- **单条规则独立开关与拖拽排序**。规则框保持纯文本一行一条（支持 `#` 注释禁用某一行），并配有顶部「启用规则 / 启用断点」总开关与实时语法行号校验，暂不拆成复杂的表格行编辑器。
- **跨 Path 前缀的笛卡尔积之外的独立规则绑定**。一条规则里的 `a.com|b.com/v1|/v2` 对列出的所有域名统一匹配列出的所有 Path 前缀；若不同域名需要完全不同的 Path 前缀，直接分写两行规则即可。
- **独立的 `jsAppend://` 网页脚本注入协议语法**。类似 Whistle `baidu.com jsAppend://baidu.com/sdk.js` 的网页脚本注入需求，直接用现有 `replace baidu.com res </body> -> <script src="https://baidu.com/sdk.js"></script></body>`（或替换 `</head>`）即可覆盖，支持多域名/多 Path 前缀 OR；不单独引入 `jsAppend://` 语法，以免为了追加脚本而将所有分块编码（Chunked）响应强行缓冲，或误伤同域名下的 JSON/JS 等非 HTML 响应。

## 待办与未完成事项整理

### 一、已评估待实现的体验与工程增强

以下事项已完成开发价值与方案评估，按优先级排列作为后续迭代待办：

1. **开发与正式环境配置隔离**（已完成）：
   - 实现：`ca.crt` 与 `ca.key` 保持共用（避免重复安装信任根证书）；在 `cfg!(all(debug_assertions, not(test)))` 或设置 `ZNIFF_DEV=1`（`make dev` 默认携带）时，配置文件与代理快照分别读写 `config.dev.toml`（默认端口 `8878`）与 `proxy-snapshot.dev.toml`，与正式版 `config.toml`（默认端口 `8877`）及 `proxy-snapshot.toml` 完全隔离。
   - 待用户确认项（可选决策，不影响当前使用）：首次生成 `config.dev.toml` 时目前默认使用空规则与空过滤以避免误触发正式版状态；后续是否需要在 `config.dev.toml` 首次创建时从 `config.toml` 单向拷贝一份 `saved_profiles` 预设列表供调试复用，可按需决定。
2. **列显示切换菜单顺序与表格列顺序一致**（已完成）：
   - 实现：列图标弹出菜单与表头右键菜单里的可切换项顺序按当前列顺序排列（默认 `序号、方法、路径、状态、间隔、耗时、大小、协议、主机、来源 IP`），与表格实际列顺序完全一致。
3. **过滤与规则配置窗口随应用窗口自适应调整**（已完成）：
   - 实现：结合 `window()` 实时尺寸与左栏宽度动态计算——过滤弹出层宽度 `clamp(window_w - list_px * 0.7 - 16, 680, 1360)`，规则编辑器高度按双栏剩余高度预算在 `84..=460` 自适应，左栏宽度上限受 `window_w - 340` 动态约束以始终保留右栏详情空间。
4. **规则配置（`SavedProfile`）导出与导入**（已完成）：
   - 实现：顶栏「过滤」弹出层底栏已提供「导出」与「导入」按钮——点击「导出」直接将 `SavedProfile` 序列化为 `.toml` 文本复制到系统剪贴板；点击「导入」展开大号多行文本输入框供粘贴 `.toml` 配置内容导入，同时支持把 `.toml` 配置文件拖到窗口上导入（同名忽略大小写覆盖，保持 16 条上限）。
5. **列表 Path 列支持切换「仅展示 Path」或「完整 URL」**（已完成）：
   - 实现：在列设置菜单底部增加 `仅路径 / 完整 URL`（`Path only / Full URL`）切换并记入 `config.toml` 的 `show_full_url`（默认仅路径）；切为完整 URL 时列标题切换为 `URL` 并直接显示 `flow.url`（隧道与失败原因显示逻辑不变），悬停提示同步跟随。
6. **列表 URL/路径与主机省略位置支持三种模式（开头 / 中间 / 结尾）**（已完成）：
   - 实现：利用 `nsh_core::prelude::EllipsizeMode` 的 `Head`（开头 `...`）、`Middle`（中间 `...`）、`Tail`（结尾 `...`），在列设置菜单底部提供省略位置切换并记入 `config.toml` 的 `ellipsize`（`head` / `middle` / `tail`），默认保持 `middle`。
7. **列表新增「来源 IP」列、列顺序自定义与点击表头排序**（已完成）：
   - 实现：支持开关「来源 IP」（`show_client_ip`，默认关）；支持拖拽表头或在列菜单内点击上移/下移调整列顺序（固定区与滚动区各自内部调序，记入 `column_order`）；支持点击表头按该列升序 `↑` / 降序 `↓` / 恢复默认顺序三态切换排序。

### 二、既有文档中尚未接上或受限的事项汇总

除上述已完成增强外，现有代码与文档中保留的未完成项及平台限制汇总如下：

1. **窗口坐标位置未记忆**：窗口宽高已自动记忆恢复（通过 `AppOptions::size`），暂不记录窗口屏幕坐标（`x, y`）。
2. **已有规则行无独立表单回填编辑**：过滤面板右栏底部的 14 项分段标签用于快速追加新规则行，修改已有规则仍直接在上方带行号与语法校验的纯文本编辑器中编辑。
3. **各平台系统代理能力边界**：
   - Linux 非 GNOME 环境无统一系统代理设置接口，开启「系统代理」后仅启动监听并在状态栏提示手动指向 `127.0.0.1:端口`。
   - Windows 仅修改当前用户的 WinINET 注册表项，不修改 WinHTTP。
   - macOS 带密码认证的系统代理无法通过 `networksetup` 读回密码，停止抓包后密码项无法自动还原；根证书信任检测在 macOS 下通过 `security find-certificate` 结合 `security verify-cert -c <ca.crt> -L` 区分「未安装」「已安装但未开启信任」与「已完全信任」。

---

## 与 Proxyman / Charles / Reqable / Whistle 的对比分析与值得增强的功能

在保持 `zniff` **单进程轻量、内存可控（环形分片落盘）、纯文本规则易分享、双栏紧凑交互**核心定位的前提下，横向对比 Proxyman、Charles、Reqable、Whistle 等主流抓包调试工具，梳理出以下**高性价比、契合现有架构、值得后续迭代增强的功能清单**：

### P1：高频痛点、投入小、收益显著（✅ 已完成）

1. **从抓包流量一键创建规则（Map Local / 快速预填规则）** — ✅ 已完成
   - **实现**：在列表右键和详情「更多」菜单中支持「本地映射」（`Map local`）与「填入断点」（`Add break`）。选中流量有响应正文时弹出保存对话框，将响应正文另存为本地文件并自动追加 `map-local <host><path> <file> [status=<code>]` 规则；响应正文为空时直接追加 `map-local <host><path> status=<code>` 规则并立即生效。

2. **独立新建请求（Scratchpad / 无选中流量时直接构造请求或粘贴 cURL）** — ✅ 已完成
   - **实现**：在顶栏「更多」菜单及右栏未选中流量的空态页面提供「构造请求」（`New request`）入口，无需先选中已有流量即可直接构造 `GET`/`POST` 等请求或粘贴 `curl` 命令解析填充并发送，发出的请求自动记入抓包会话列表。

3. **URL 查询参数改写规则（`query` 规则动词）** — ✅ 已完成
   - **实现**：新增修饰类规则 `query <主机[/路径]> set <名> <值>`（或 `<名>=<值>`）与 `query <主机[/路径]> del <名>`，以及快速添加规则栏中的「改参」（`Query`）标签，在代理转发前自动解析并重构 URL query，正确处理 URL 编码与分隔符。

4. **`map-local` 内联状态码空响应与带引号路径/文本支持** — ✅ 已完成
   - **实现**：支持 `map-local <主机[/路径]> status=503 [delay=...]`（省略文件路径直接返回指定状态码与空正文）；规则解析器全面支持双引号 `"..."` 包裹含空格的本地文件路径、`replace` 匹配原文与替换目标、`header` 值与 `query` 参数值，并在快速添加规则时自动为含空格词补齐双引号。

5. **`multipart/form-data` 请求体结构化预览** — ✅ 已完成
   - **实现**：在「美化」模式下按 `boundary` 解析 `multipart/form-data` 各分片，在请求 body 卡片中以结构化键值表格展示每个分片的 `name`、文件元数据（`filename`、`Content-Type`、字节大小）及文本内容预览；切回「原文」模式仍查看原始报文或十六进制。

---

### P2：深度调试体验增强（✅ 已完成）

6. **详情 Body 内关键字搜索与高亮定位** — ✅ 已完成
   - **实现**：在请求/响应 Body 卡片标题栏右侧提供内联搜索框与命中计数角标（点击角标一键清空），实时按关键词（忽略大小写）筛选匹配行并标注 `[L0001]` 四位行号，对表单/Multipart 视图同步按字段名或值实时过滤。

7. **本地客户端进程名识别（Process / App Attribution）** — ✅ 已完成
   - **实现**：当客户端地址为本机回环（`127.0.0.1` / `::1`）时，在后台线程池（`spawn_blocking`）异步解析客户端源端口所属的本地进程名（Linux `/proc/net/tcp*` + `/proc/<pid>/fd`、macOS `lsof`、Windows `netstat`/`tasklist`）并带 15 秒端口缓存，在「来源 IP」列展示为 `127.0.0.1 (curl)` 等格式（绝不阻塞代理转发），同时过滤语法支持 `app:<进程名>` / `proc:<进程名>`。

8. **断点并发队列（Breakpoint Queue）** — ✅ 已完成
   - **实现**：将断点槽位升级为最多 8 条的有界 FIFO 队列（`BREAK_QUEUE_CAP = 8`，超出时自动放行队首最旧断点以防死锁），队内等待的断点在轮到队首前自动保持 60 秒超时计时器新鲜；顶部断点栏在排队数 `> 1` 时显示 `(1 / N)` 进度并提供「全部放行」（`Continue all`）按钮，同时按队列顺序排出 `#id 请求/响应` 胶囊；点一条把它提到队首并选中，放行和编辑重发作用在队首，60 秒计时从新的队首重新计算。

9. **流量备注（Comment / Note）与「仅清空未星标流量」** — ✅ 已完成
   - **实现**：支持通过列表右键或详情「更多」菜单点击「备注」（`Note`）为流量添加备注（摘要卡片显示可点击编辑的 `# 备注` 徽章，路径列追加 `· #备注` 展示，过滤框支持 `note:<关键词>` / `comment:<关键词>` 或裸词检索，HAR 1.2 导入导出完整读写标准 `entry.comment` 字段）；顶栏「更多」菜单新增「清空未星标」（`Clear unstarred`），一键清理普通流量并保留星标流量。

10. **GraphQL 与 Protobuf / MessagePack 零配置可读预览** — ✅ 已完成
    - **实现**：美化模式下自动识别包含 `query` 字段的 GraphQL JSON 请求，将 `operationName`、缩进排版后的多行 GraphQL 查询语句与 `variables` JSON 分块展示；美化模式下对 Protobuf（`application/x-protobuf`、`application/protobuf`、`application/grpc*` 含 gRPC 5 字节帧头）提供无 Schema 的 Wire Tag 解码视图（展示 `field #N (varint/fixed64/fixed32/string/bytes/message)` 及嵌套消息），对 MessagePack（`application/msgpack`、`application/x-msgpack`）自动解码为格式化 JSON 预览，切换「原文」仍可查看 16/32 字节十六进制视图。

---

### P3：进阶压测与诊断辅助（✅ 已完成）

11. **重复重放 / 批量重放（Repeat Advanced）** — ✅ 已完成
    - **实现**：在列表右键与详情「更多」菜单中提供「批量重放…」（`Repeat…`），在详情摘要卡片内展开内联批量重放配置栏，支持指定重放次数（`1..=100`，默认 `5`，含 `5×` / `10×` / `20×` 快捷预设）与间隔毫秒数（`0..=60000`，默认 `200 ms`），在后台按序逐条重放并记录至会话列表，完成后状态栏反馈 `已批量重放 #id ×N`。

12. **当前列表聚合统计（Filtered Summary）** — ✅ 已完成
    - **实现**：新增 `session::FilteredStats` 实时汇总当前筛选集合的总请求数、总传输字节（及上行/下行拆分）、平均耗时、状态码分布（`2xx` / `3xx` / `4xx` / `5xx` / `ERR` / `进行中` 语义色角标）与最慢请求；在右栏未选中流量的空态顶部展示「当前列表统计」卡片（点击最慢请求直接跳转选中该条流量），顶栏「清空」按钮旁提供「统计」（`Summary`）按钮（或点击会话条数角标、按 `Esc` 键）一键返回统计视图。

13. **客户端代理环境与证书信任一键诊断** — ✅ 已完成
    - **实现**：「复制代理环境」（`present::proxy_env_export`）扩展覆盖 `http_proxy` / `https_proxy` / `all_proxy`（含大小写）、`SSL_CERT_FILE`、`NODE_EXTRA_CA_CERTS`、`REQUESTS_CA_BUNDLE`、`CURL_CA_BUNDLE`、`GIT_SSL_CAINFO`、`CARGO_HTTP_CAINFO`、`DENO_CERT` 与 `JAVA_TOOL_OPTIONS`；新增 `ca::CaStatus`（`Trusted` / `InstalledUntrusted` / `NotInstalled`），macOS 下结合 `security find-certificate` 区分「证书：已信任」「证书：未开启信任」（警告色提示并同时提供「信任证书」与「移除证书」）和「证书：未安装」。

14. **弱网带宽限流（`throttle`）、域名真实证书挂载（`cert`）、并发批量重放与自动检测升级** — ✅ 已完成
    - **实现**：
      - **带宽限流（`throttle`）**：支持 `throttle <主机[/路径]> <预设|速率>`（预设 `2g`/`slow-3g`、`3g`、`4g`/`slow-4g`，或 `down=50kbps up=20kbps`），在代理流转发层按 4 KB 分片对请求上传与响应下发进行流式速率限制。
      - **真实域名证书挂载（`cert`）**：支持 `cert <主机> <cert.crt/.pem> [key.key]` 规则及数据目录 `certs/` 自动匹配（含泛域名 `_wildcard.<domain>`），使用真实证书链与私钥完成 MITM 握手。
      - **并发批量重放与消息筛选**：批量重放支持并发数（`1..=20`）与 `repeat:<id>` 追踪标记；WebSocket/SSE 消息卡片支持方向（`↑`/`↓`）、帧类型与关键词过滤。
      - **自动升级与原地重启（`updater`）**：后台定时检测 GitHub Releases（支持 Gitee 镜像回退），流式下载并校验 `SHA256SUMS`，原地替换 `.app` / 可执行文件并自动释放抓包端口后重启恢复。

---

### P4：高性价比、立竿见影的差异化增强（✅ 已完成）

15. **单条请求连接阶段耗时明细（Timing Breakdown & Connection Metadata）** — ✅ 已完成
    - **实现**：
      - 在 `proxy.rs` 的 `dial_timed` 与 `connect_origin` 中精细采集 `dns_ms`、`tcp_ms`、`tls_ms`，结合 `response_at` 与 `finished_at` 计算 `wait_ms`（TTFB 服务端处理等待）与 `download_ms`（响应体传输耗时）。
      - 在 `session::Flow` 中附加 `ConnInfo` 连接元数据：支持记录连接复用状态 `reused`（命中 HTTP/1.1 空闲池或 HTTP/2 多路复用后续流时标记为复用）、远端真实地址 `server_addr` 与协商出的 TLS 版本 `tls_version`（`TLS 1.3` / `TLS 1.2`）。
      - 在右栏详情摘要卡片下方渲染可视化连接标签（`新建连接` / `复用连接`、TLS 版本、远端 `IP:Port`）与多色比例耗时分段条（`DNS`、`TCP`、`TLS`、`等待`、`下载`），并在 HAR 1.2 导入导出中完整读写标准 `timings.{dns, connect, ssl, wait, receive}` 与 `serverIPAddress`。

16. **内置 MCP Server 与 Rust / Go 代码生成（AI 联调降维打击）** — ✅ 已完成
    - **实现**：
      - **Code Export 扩展（`present.rs`）**：在详情「更多」菜单中新增复制 **Rust (`reqwest`)** 请求代码（`present::rust_reqwest`）、**Go (`net/http`)** 请求代码（`present::go_http`），以及从响应 JSON 一键推导 **Rust `#[derive(Debug, Clone, Serialize, Deserialize)] struct`**（`present::rust_serde_struct`，自动将字段名转为 `snake_case`、处理 Rust 保留字 `r#` 与数字开头标识符并补充 `#[serde(rename = "...")]`）。
      - **内置本地 MCP 接口（`mcp.rs` + `proxy.rs`）**：在代理监听端口新增仅限本机回环（`peer.ip().is_loopback()`）访问的 `POST /mcp` JSON-RPC 2.0 路由（不记入抓包会话列表），并支持命令行 `zniff --mcp [--port <port>]` stdio 桥接模式；暴露 `list_flows`（直接复用 `zniff` 过滤表达式）、`get_flow_detail`（含头、完整正文、连接耗时明细与 WebSocket 消息）、`replay_flow`（支持重放已有流量或构造新请求并返回新流量详情）、`get_rules` 与 `set_rules`（实时读取与校验/更新规则及直通名单）共 5 个 MCP Tool。顶栏「更多」菜单新增「复制 MCP 配置」（`Copy MCP config`）一键复制 Claude Code / Cursor / Antigravity 可直接粘贴的 `mcpServers.zniff` 配置 JSON。

17. **快捷语义规则：`no-cache`（禁用缓存）与 `block`（阻断连接）** — ✅ 已完成
    - **实现**：
      - **`no-cache` / `nocache`（禁用缓存）**：叠加类规则 `no-cache <主机[/路径]> [op=<操作名>]`，匹配时自动从请求头剥离 `If-Modified-Since` 与 `If-None-Match` 并写入 `Cache-Control: no-cache, no-store, must-revalidate`、`Pragma: no-cache`，同时从响应头剥离 `ETag` 与 `Last-Modified` 并写入 `Cache-Control: no-cache, no-store, must-revalidate`、`Pragma: no-cache`、`Expires: 0`，避免 `304 Not Modified` 干扰抓包查看正文。
      - **`block`（阻断连接）**：终点类规则 `block <主机[/路径]> [drop|reset|status=<code,默认403>] [op=<操作名>]`，不拨号源站；`status` 模式立即返回指定拦截状态码（默认 `403 Forbidden`）与空正文，`drop` / `reset` 模式直接关闭底层连接或重置流并在流量上记录 `规则阻断 (连接已断开)`。过滤弹出层快速添加区同步提供「禁缓存」与「阻断」（含「状态码 / 断开」切换）。

18. **macOS iOS Simulator 一键注入证书 & Linux 系统级证书指引** — ✅ 已完成
    - **实现**：
      - **iOS 模拟器证书一键安装（macOS）**：在顶栏「更多」菜单证书区域新增「安装证书到 iOS 模拟器」（`Install on iOS Simulator`），后台执行 `xcrun simctl list devices booted` 枚举当前所有已启动（`Booted`）的模拟器设备 UDID，并逐台调用 `xcrun simctl keychain <udid> add-root-cert <ca.crt>` 写入并信任根证书，再用 `xcrun simctl spawn <udid> defaults write -g` 把 HTTP/HTTPS 代理设为 `127.0.0.1:<当前监听端口>`。这次设过的模拟器记在进程里，停止抓包或进程退出时尽力把 `HTTPEnable` / `HTTPSEnable` 写成 0 并删掉代理主机和端口。未检测到运行中模拟器时状态栏给出明确提示。真机代理仍要用户自己设置。
      - **Linux 系统级证书命令一键复制**：在 Linux 平台「更多」菜单证书区域新增「复制系统证书命令」（`Copy system CA command`），自动检测 `/etc/os-release` 与发行版标识文件（Arch/Manjaro、Debian/Ubuntu、Fedora/RHEL/Rocky、openSUSE、Alpine）并生成对应的系统级 Trust Store 安装命令（如 `sudo trust anchor --store ...`、`sudo cp ... /usr/local/share/ca-certificates/zniff-ca.crt && sudo update-ca-certificates` 或 `sudo update-ca-trust`）复制到剪贴板。

19. **可选折叠的 Domain / App 快速导航抽屉（Sidebar Tree / Filter Facets）** — ✅ 已完成
    - **实现**：
      - 在 `session::Session::facets` 中实时聚合当前会话中的「域名（`Domains`）」与「应用 / 来源（`Apps / Clients`）」（从 `127.0.0.1 (进程名)` 提取应用进程名或保留远端来源 IP，含空格进程名自动映射为下划线并在 `app:` 过滤中双向匹配），按流量条数降序排列并统计各分面的异常/报错条数。
      - 在左栏搜索框右侧（列设置按钮旁）新增「域名与应用导航」折叠开关，展开后在列表上方显示双标签分面导航抽屉（`域名 (N)` / `应用 / 来源 (M)`）；点击分面胶囊即可一键切换/追加 `host:xxx` 或 `app:xxx` OR 过滤词，点击域名胶囊右侧 `×` 可一键切换 `mute:xxx` 屏蔽。

---

### P5：高阶扩展方向（按需演进）

20. **动态脚本改写（Scripting / Dynamic Rules）**
    - **场景**：针对 20% 静态 DSL 无法覆盖的动态计算场景——例如修改请求 Body 后需用 `HMAC-SHA256` 动态重算请求头中的 `X-Signature` / `X-Timestamp`，或对自定义 AES/RSA 加密的请求/响应报文进行透明加解密展示。
    - **架构评估**：为避免给主进程引入重量级运行时，可评估两种演进路径：
      1. **轻量嵌入式引擎**：按需引入极轻量的 `rquickjs` 或 `mlua`，支持 `script <主机[/路径]> <path.js/.lua>`，在受控沙箱与超时限制内暴露 `onRequest(ctx)` / `onResponse(ctx)` 钩子及内置 `crypto`（HMAC/SHA256/AES/Base64）工具函数；
      2. **外部 Filter 进程 / 本地 Webhook 钩子**：无需内嵌语言虚拟机，允许将匹配流量通过 stdin/stdout 或本地 Unix Socket 交给用户自己的脚本处理。
