# zniff 已知限制

P0 到 P3 里点名的能力已经接上。下面是仍然接受的缺口，不挡当前使用。

## 仍然不做

- HTTP/3、QUIC。UDP 不经过 HTTP 代理。
- Firefox 的证书库。Firefox 默认也不代理 localhost。验收浏览器是 Chrome、Safari、Edge。
- 透明代理、JSON 树、SQLite、cyberapi、Web 界面。
- 会话只在内存里。重启清空。单条 body 解压后最多 1 MB，整段会话最多 256 MB，超出丢掉最旧的整条 flow。线字节和 WebSocket 消息文本不计入这两个上限。

## 平台

- 系统代理默认关闭。只有「更多」里的「系统代理」为开时，开始抓包才会改系统代理。启动不会打开它。
- Linux 非 GNOME：开了「系统代理」也改不了，界面给出监听地址，需要自己指到 `127.0.0.1:端口`。监听仍会起来。
- Windows 只改当前用户的 WinINET，不改 WinHTTP。
- macOS `security add-trusted-cert` 可能要用户点确认。`security find-certificate` 只说明证书在登录钥匙串里，不保证 SSL 信任策略已经生效。
- 带密码的系统代理：`networksetup` 读不回密码，停止后可能恢复不完整。
- Windows、Linux 上「移除证书」还没接上。macOS 用 `security delete-certificate`。

## 代理行为

- 没配上游时，源站连接直连，不走系统代理。`config.toml` 里配了 `upstream_proxy`（`host:port`）后，明文用绝对形式，TLS 先对上游发 `CONNECT`。
- 明文 HTTP 省略 `Connection` 时补 `Connection: keep-alive`。body 读完、双方都没要求关闭、状态不是 101 时才放回池。同一个键最多 4 条，键超过 32 就清空。HTTP/1 的 TLS 不进这个池。重放强制 `Connection: close`，不进池。
- HTTP/2 在一条客户端连接上复用源站连接，另外缓存最多 16 条发送端。`ready` 失败或发送失败就丢掉。连接驱动结束时不会立刻删缓存。
- 源站 TLS 探测失败后会关掉那条探测连接，另开一条 TCP，再把缓冲的 ClientHello 放进去。探测失败的同一条 TCP 不能复用。
- 源站没选 ALPN 时按 HTTP/1.1 记。
- ServerHello 之前失败改成 TCP 隧道，只记目标和原因。ServerHello 之后的解析失败只记错误，不能退回隧道。
- ServerHello 之后客户端断开，界面尽量记成「证书被拒绝」。这是根据连接被重置做的判断，不是证书钉扎的专用检测。直通名单可以避开钉扎主机。
- HTTP/2 这一层只表示两边用源站选定的协议。不解析帧、HPACK、trailer、gRPC。
- 界面里非 UTF-8 body 只显示前 4 KB 十六进制，UTF-8 只显示前 256 KB。记录本身仍保留最多 1 MB。复制用的是记录原文。
- WebSocket 从旁路解析帧并立刻转发。转发握手时去掉 `Sec-WebSocket-Extensions`，避免谈成 `permessage-deflate`。非法帧或仍然带扩展位（`RSV1 != 0`）只停止解析后续帧，不切断对拷。消息最多 200 条或 256 KB 文本。`Content-Type` 含 `event-stream` 且没有 WebSocket 消息时，按 SSE 事件列出。
- 规则按写下来的顺序，第一条命中。明文在拨号前应用，HTTPS 在源站连接打开后应用，WebSocket 和 `CONNECT` 跳过。Map Remote 换了主机就另开连接。断点只拦请求不拦响应，单槽位保存当前挂起请求（新命中的断点会顶替并自动放行上一条），等 60 秒，超时当放行。重放不套规则和断点。规则里的文件路径不能有空格。
- 直通名单在 ClientHello 之后看 SNI，没有 SNI 就用 CONNECT 主机。`*.example.com` 匹配子域，不匹配顶点。`only:` 是白名单。`*` 全部直通。空名单全部解密。
- `GET` 或 `HEAD` 的路径去掉 query 后是 `/zniff-ca.crt` 时直接返回根证书 PEM。直接访问 `/`，或路径是 `/zniff`，返回手机安装页。`Accept-Language` 首段不是 `zh` 时安装页是英文，空的或 `zh` 仍是中文。代理请求里的 `http://其他主机/` 仍转发出去。
- 监听默认 `127.0.0.1:8877`。端口和局域网开关记在 `config.toml` 的 `listen_port`、`listen_all`，`0` 拒绝，绑不上就报错，不悄悄换端口。`listen_all` 打开后绑 `0.0.0.0`。监听地址是 `0.0.0.0` 之后，底栏「扫码」点开才显示安装页二维码。默认地址是访问公网的那张 IPv4；隧道和 `198.18.0.0/15` 让位给物理网卡。多张网卡可改选。没有系统文件选择框。图片预览只做 png、jpeg、webp、gif，且响应已结束、非空、未截断。SVG 不预览。

## 已接上，不再当作待办

这些有库测试：

- 清空会话。清列表、字节和丢弃计数，不清下一条 id。切换选中行或点「清空」会收起「编辑重发」并清掉编辑框。
- 复制 URL、cURL、请求 body、响应 body。cURL 跳过 `Host` 和 `Content-Length`。非 UTF-8 复制十六进制。二进制请求 body 不写进 cURL。
- 记住监听端口。摘要里有开始时间、TTFB、总耗时、请求和响应线字节。时区用启动时的 `date +%z`，否则 UTC。HAR 导入按 `timings.wait`、`time` 和 `bodySize` 回填 `response_at`、`finished_at`、`request_bytes`、`response_bytes`。`bodySize` 缺失或为负时用已还原的 body 长度。
- JSON、表单、XML、HTML 默认原文，可切「美化」。查询参数单独成表。
- 过滤是 AND 词：方法精确、状态精确或 `1xx`–`5xx`、`失败`、`type:` / `ct:`、`body:`、`header:`，其余匹配主机或路径。`body:` 搜请求和响应 body，`header:` 搜头名和头值。
- 「编辑重发」可改方法、URL、头和 body。发送时按新 body 重算 `Content-Length`。URL 改了就丢掉旧的 `Host`。
- 直通名单、上游绝对形式、明文连接池、WebSocket 文本帧、握手去掉 `Sec-WebSocket-Extensions`、Map Local、断点丢弃、HTTPS 上的 Map Local / Map Remote / 断点放行、HTTP/2 发送端缓存命中、HTTP/2 Map Local、HTTP/2 断点丢弃、重放改方法 URL 头和 body、`/zniff-ca.crt`、安装页 `/zniff` 有代理往返测试。HAR、规则语法、SSE、美化、`body:` / `header:` 有单元测试。
