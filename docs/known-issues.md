# zniff 已知限制

P0 到 P3 里点名的能力已经接上。下面是仍然接受的缺口，不挡当前使用。

## 仍然不做

- HTTP/3、QUIC。UDP 不经过 HTTP 代理。
- Firefox 的证书库。Firefox 默认也不代理 localhost。验收浏览器是 Chrome、Safari、Edge。
- 透明代理、JSON 树、SQLite、cyberapi、Web 界面。
- 会话只在内存里。重启清空。单条 body 解压后最多 1 MB，整段会话最多 256 MB，超出丢掉最旧的未星标 flow。星标最多 32 条，不参与淘汰，因此星标集合可以让内存超过 256 MB。线字节和 WebSocket 消息文本不计入这两个上限。过滤不写入配置。

## 平台

- 系统代理默认关闭。只有「更多」里的「系统代理」为开时，开始抓包才会改系统代理。启动不会打开它。
- Linux 非 GNOME：开了「系统代理」也改不了，界面给出监听地址，需要自己指到 `127.0.0.1:端口`。监听仍会起来。
- Windows 只改当前用户的 WinINET，不改 WinHTTP。
- macOS `security add-trusted-cert` 可能要用户点确认。`security find-certificate` 只说明证书在登录钥匙串里，不保证 SSL 信任策略已经生效。
- 带密码的系统代理：`networksetup` 读不回密码，停止后可能恢复不完整。
- Windows、Linux 上「移除证书」还没接上。macOS 用 `security delete-certificate`。
- 窗口位置和大小不记。nsh 0.2 的 `AppOptions` 只有初始 `size`，没有坐标。Rust 组件能调的窗口接口是 `set_title_bar`、`title_bar`、`close_window`。`Window.setFrame` 在运行时里是 `pub(crate)`，`desktop_command` 也是，应用读不到当前窗口框，也发不出移动命令。只保存宽高同样做不到，因为关窗口时读不回用户拖过的尺寸。不升 nsh，也不改平台 crate。启动尺寸仍是 `app/src/main.rs` 里的 `(1200, 760)`。

## 代理行为

- 没配上游时，源站连接直连，不走系统代理。`config.toml` 里配了 `upstream_proxy`（`host:port`）后，明文用绝对形式，TLS 先对上游发 `CONNECT`。
- 明文 HTTP 省略 `Connection` 时补 `Connection: keep-alive`。body 读完、双方都没要求关闭、状态不是 101 时才放回池。同一个键最多 4 条，键超过 32 就清空。HTTP/1 的 TLS 不进这个池。重放强制 `Connection: close`，不进池。
- HTTP/2 在一条客户端连接上复用源站连接，另外缓存最多 16 条发送端。`ready` 失败或发送失败就丢掉。连接驱动结束时不会立刻删缓存。
- 源站 TLS 探测失败后会关掉那条探测连接，另开一条 TCP，再把缓冲的 ClientHello 放进去。探测失败的同一条 TCP 不能复用。
- 源站没选 ALPN 时按 HTTP/1.1 记。
- ServerHello 之前失败改成 TCP 隧道，只记目标和原因。ServerHello 之后的解析失败只记错误，不能退回隧道。
- ServerHello 之后客户端断开，界面尽量记成「证书被拒绝」。这是根据连接被重置做的判断，不是证书钉扎的专用检测。直通名单可以避开钉扎主机。
- HTTP/2 这一层只表示两边用源站选定的协议。不解析帧、HPACK、trailer、gRPC。
- 界面里非 UTF-8 body 只显示前 4 KB 十六进制，UTF-8 只显示前 256 KB，截在字符边界上。记录本身仍保留最多 1 MB。复制用的是记录原文。左栏按到达时间排列，最新在最下面；离底部不超过一行时跟着新流量，再往上翻就停在当前内容。响应 body 在内层已经 `is_end_stream()`，或 `RecordingBody` 被丢弃时记完成。空响应（例如 `304`）和最后一帧不会停在「进行中」。超过 1 秒的时间列用警告色。主机列和路径列保持正常颜色。
- WebSocket 从旁路解析帧并立刻转发。转发握手时去掉 `Sec-WebSocket-Extensions`，避免谈成 `permessage-deflate`。非法帧或仍然带扩展位（`RSV1 != 0`）只停止解析后续帧，不切断对拷。消息最多 200 条或 256 KB 文本。`Content-Type` 含 `event-stream` 且没有 WebSocket 消息时，按 SSE 事件列出。
- 规则里 `map-local`、`map-remote`、`break`、`break-res` 是终点，按写下来的顺序第一条命中。`delay`、`header`、`cors` 不占这条名额，匹配到的都会叠加上去，包括写在终点后面的行。`delay` 最长 30 秒。`map-local` 可写 `status=429`，省略时是 200。`header` 用 `req` 或 `res`，再写 `set` 或 `del`。`cors` 给响应补跨域头；`OPTIONS` 且没有终点规则时直接回 204，不拨源站。明文在拨号前应用，HTTPS 在源站连接打开后应用，WebSocket 和 `CONNECT` 跳过。Map Remote 换了主机就另开连接。请求断点和响应断点共用一个槽位，新命中会顶替并自动放行上一条，等 60 秒，超时当放行。响应断点先缓冲 body，最多 1 MB。已经写明的 `Content-Length` 超过 1 MB 时不暂停，记「响应断点跳过」后照常转发。分块或未知长度读过 1 MB 时，这条响应记失败。放行时可以改状态码、头和 body。重放不套规则和断点。规则里的文件路径不能有空格。
- 直通名单在 ClientHello 之后看 SNI，没有 SNI 就用 CONNECT 主机。`*.example.com` 匹配子域，不匹配顶点。`only:` 是白名单。`*` 全部直通。空名单全部解密。
- `GET` 或 `HEAD` 的路径去掉 query 后是 `/zniff-ca.crt` 时直接返回根证书 PEM。直接访问 `/`，或路径是 `/zniff`，返回手机安装页。`Accept-Language` 首段不是 `zh` 时安装页是英文，空的或 `zh` 仍是中文。代理请求里的 `http://其他主机/` 仍转发出去。
- 监听默认 `127.0.0.1:8877`。端口和局域网开关记在 `config.toml` 的 `listen_port`、`listen_all`，`0` 拒绝，绑不上就报错，不悄悄换端口。`listen_all` 打开后绑 `0.0.0.0`。监听地址是 `0.0.0.0` 之后，底栏「扫码」点开才显示安装页二维码。默认地址是访问公网的那张 IPv4；隧道和 `198.18.0.0/15` 让位给物理网卡。多张网卡可改选。没有系统文件选择框。图片预览只做 png、jpeg、webp、gif，且响应已结束、非空、未截断。SVG 不预览。

## 已接上，不再当作待办

这些有库测试：

- 清空会话。按钮在顶栏「更多」里，紧挨「导出」，清的是整个会话。清列表、字节和丢弃计数，不清下一条 id。切换选中行或点「清空」会收起「编辑重发」并清掉编辑框。
- 复制 URL、cURL、请求 body、响应 body。cURL 跳过 `Host` 和 `Content-Length`。非 UTF-8 复制十六进制。二进制请求 body 不写进 cURL。
- 记住监听端口。摘要里有开始时间、TTFB、总耗时、请求和响应线字节。时区用启动时的 `date +%z`，否则 UTC。HAR 导入按 `timings.wait`、`time` 和 `bodySize` 回填 `response_at`、`finished_at`、`request_bytes`、`response_bytes`。`bodySize` 缺失或为负时用已还原的 body 长度。
- JSON、表单、XML、HTML 默认美化，可切「原文」。复制和另存仍用记录原文。查询参数单独成表。另存写到下载目录的 `zniff` 文件夹（Linux 读 `~/.config/user-dirs.dirs` 的 `XDG_DOWNLOAD_DIR`，否则 `~/Downloads`），状态栏给出完整路径。HAR 仍导出到数据目录的 `exports/`。
- 过滤是 AND 词：方法精确、状态精确或 `1xx`–`5xx`、`失败`、`type:` / `ct:`、`body:`、`header:`、`pin:`、`mute:` 或 `-词`，其余匹配主机或路径。`body:` 搜请求和响应 body，`header:` 搜头名和头值。`pin:` 只留星标。`mute:` 和 `-词` 排除主机或路径里带这个词的行。方法行右侧「更多」里「只看」把过滤换成当前主机，「屏蔽」追加 `-主机`（主机带冒号时用 `mute:`）。复制、另存、星标、基准、重放、编辑重发和美化也在这个菜单里。
- 「编辑重发」可改方法、URL、头和 body。URL 框以 `curl` 开头时，按 `-X`、`-H`、`--url`、`-d` / `--data*`、`-A`、`-e`、`-b` 填进这些框；带 body 又没写方法时当成 `POST`。发送时按新 body 重算 `Content-Length`。URL 改了就丢掉旧的 `Host`。停在响应断点上再点「编辑重发」，改的是状态码、响应头和响应 body，由断点条的「放行」送出。状态码只接受 100–599。
- 复制还有 `fetch`、Python `requests`，以及响应 JSON 对象或数组的浅层 TypeScript `interface`。「更多」里「复制代理环境」复制 `http_proxy`、`https_proxy`、`SSL_CERT_FILE`、`NODE_EXTRA_CA_CERTS`、`REQUESTS_CA_BUNDLE`、`CURL_CA_BUNDLE`、`GIT_SSL_CAINFO`，证书路径指向数据目录的 `ca.crt`，端口用当前记住的监听端口。请求头和查询值里能认出的 JWT、10 位或 13 位时间戳、可打印 Base64 会附在原值下面。两条流量可以用「基准」做行级对比。超过 1 秒的时间列用警告色。星标在主机名前有蓝点。
- 直通名单、上游绝对形式、明文连接池、WebSocket 文本帧、握手去掉 `Sec-WebSocket-Extensions`、Map Local、断点丢弃、HTTPS 上的 Map Local / Map Remote / 断点放行、HTTP/2 发送端缓存命中、HTTP/2 Map Local、HTTP/2 断点丢弃、重放改方法 URL 头和 body、`/zniff-ca.crt`、安装页 `/zniff` 有代理往返测试。HAR、规则语法、SSE、美化、`body:` / `header:`、`status=` / `delay` / `header` / `cors`、响应断点改写、星标不淘汰、环境变量、`fetch` / Python / TypeScript、cURL 解析、JWT 提示和行级 diff 有单元测试。

## 界面

左栏默认宽 560，可拖到 280–1100。方法、状态、路径、间隔、主机、协议、大小、URL、头和 body 用等宽。按钮和句子用界面字体。英文界面的按钮和说明不改成等宽。

- 列表默认列是方法、状态、路径、间隔、主机。方法列始终在。表头右侧「列」开关状态、协议、路径、间隔、主机、大小，写进 `config.toml`：`show_status`、`show_path`、`show_gap`、`show_host` 默认开，`show_protocol`、`show_size` 默认关。旧文件没有这些键时仍是现在这几列。间隔是距到达顺序里上一条的时间，首条 `+0 ms`（`+45 ms`、`+1.24 s`，再长用分和小时）。被过滤掉的行仍计入下一条的间隔，所以藏起一行不会改两边剩下的数字。悬停显示本地 `HH:MM:SS.mmm`。详情摘要用同一绝对时间。路径可见时占剩余宽度；藏起路径后，空出来的宽度不挪给别的列。主机是较窄的固定列。路径和主机超出时中间省略，悬停显示全文。协议列用和摘要徽章相同的名字，大小列是响应的线字节。左栏与右栏之间可拖，松开后把宽度记到 `list_width`，限制在 280 到 1100，默认 560。
- 详情在摘要、消息和对比之后是响应 body、响应头、查询参数、请求头、请求 body。请求 body 为空且没有截断或解压说明时不画那张卡片。响应 body 没有正文、没有说明、美化模式下也没有图片时同样不画。图片在美化时画在响应 body 卡片里，并写上字节数；原文显示十六进制。两张 body 卡片的标题上都有「美化 / 原文」，菜单里仍可切换。美化时 `Cookie` 和 `Set-Cookie` 按 `;` 拆开，每个键一行，名字是 `Cookie · session` 这种；没有等号的属性（`HttpOnly`、`Secure`）值留空。原文和复制仍是整段。美化时请求头和响应头按名字忽略大小写升序，同名保持原来的先后和大小写；原文、cURL 和复制不排序。

## 基于实际抓包截图的体验改进项（待办）

结合真实业务抓包截图暴露出的问题与交互反馈，以下改进具有明确实用价值：

9. **截图暴露的其他界面细节打磨**
   - **`CONNECT` 失败行的列表展示优化**：截图中 `CONNECT` 握手失败行在状态列显示红色省略号 `…`、路径列显示 `/`，表意不明。应将状态明确显示为 `ERR` 或 `—`，路径列直接展示简短原因（如 `TLS 握手失败` / `证书被拒绝`）而非 `/`。
   - **右栏卡片可折叠（或请求/响应分组）与顺序优化**：截图中「响应 body」和「响应头」占满首屏后，「查询参数」和「请求头（20 项）」被挤出可视区。支持点击卡片标题折叠/展开，并将与 URL 紧相关的「查询参数」提到更靠前位置。
   - **底栏减负与拖拽导入 HAR**：底栏右侧常驻的 `import.har` 输入框与「导入」按钮属于低频操作，视觉上显得拥挤，且与顶栏「更多」里的「导出」割裂。可收进顶栏「更多」菜单或支持直接拖拽 `.har` 文件到窗口导入（`on_file_drop`），让底栏保持干净。

## 后续吸引力增强方向（这一轮跳过）

上面第 1 到第 4 组里，环境变量、响应断点、状态码和延迟、头改写、CORS、行级对比、只看 / 屏蔽 / 星标、慢请求在时间列着色、JWT 与编码提示、`fetch` / Python / TypeScript、粘贴 cURL，都已经接上。下面这些不做：

- **TCP 建连和 TLS 握手分段计时**。连接池命中时没有新的握手。摘要里已经有 TTFB 和总耗时。慢请求用时间列上超过 1 秒的警告色代替分段计时。
- **丢包和带宽限制**。`delay`（最长 30 秒）和 `status=` 覆盖 Loading、超时和错误边界。按字节限速或随机丢包是另一套转发，不放进现在的规则。
- **JSON 树对比**。对比是 URL、请求头、请求 body、响应头、响应 body 的行级差异，每边最多 200 行。不做可折叠的 JSON 树。
- **超过 1 MB 的响应改写**。响应断点的缓冲上限就是单条 body 上限。更大的响应只转发，不在界面里改。
- **完整 cURL 语法**。不解析换行续行、`-G`、`--json`、`--form` 和多段 body。认不出的旗标忽略。
- **非 JSON 的 TypeScript**。响应不是 JSON 对象或数组时，「类型」提示不可用，不猜表单或 XML。
