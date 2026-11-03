# zniff

桌面 HTTP(S) 抓包工具。界面是 nsh 的 Rust 根组件，代理和窗口在同一个进程里。产品名是 `zniff`，全小写。

`zniff-dev-plan.md` 只保留了旧计划的名字。它写的是 gpui、hudsucker、多 crate、SQLite 和 cyberapi，不是现在这棵树。改代码以 `app/` 和 `docs/known-issues.md` 为准。

## 布局

- `app/`：唯一的 crate，自带 `[workspace]`。不要把仓库根也做成包含 `app/` 的 workspace。
- `app/nsh.json`：`root` 是 `App`。入口是 `app/src/main.rs`，根组件是 `app/src/lib.rs` 里的 `App`。
- `app/src/proxy.rs`：CONNECT、TLS、HTTP/1.1、HTTP/2、记录。
- `app/src/session.rs`：内存会话。
- `app/src/ca.rs`：根 CA 和叶子证书。
- `app/src/sysproxy.rs`：系统代理的记住、改写和恢复。
- `app/src/client_hello.rs`、`app/src/decompress.rs`：ClientHello 和 `Content-Encoding`。
- `app/src/bypass.rs`、`app/src/rules.rs`、`app/src/har.rs`、`app/src/present.rs`、`app/src/ws.rs`：直通名单、Map/断点、HAR、展示格式、WebSocket 帧。
- `app/src/i18n.rs`、`app/src/icon.rs`：界面语言和单色描边图标。
- `app/src/config.rs`：用户配置，数据目录里的一份 `config.toml`。
- `docs/known-issues.md`：已经接受的缺口。新的缺口补在这里，不要为了绕开它停掉当前开发。
- 依赖 nsh 0.3。edition 是 2024，`gen` 是保留字。原语从 `nsh_core::prelude` 引入，不再使用 `nsh_core::ui`。窗口位置和大小不写入配置：`AppOptions` 只有初始 `size`，公开的窗口接口是 `set_title_bar`、`title_bar`、`close_window`。启动尺寸仍是 `app/src/main.rs` 里的 `(1200, 760)`。
- 写组件之前先看 [nsh elements](https://nsh.dev/elements/) 有没有现成的。原语用 `nsh_core::prelude`，基础组件用 `nsh_base`。

## 这一刀的边界

不要加这些，除非用户明确要求：HTTP/3、QUIC、透明代理、JSON 树、SQLite、cyberapi、Firefox 证书库、Web 界面。手机安装页 `/zniff` 是唯一对外的网页。

- 界面只有两栏。左栏默认宽 560，拖分割条可调到 280–1100，松开后记在 `config.toml` 的 `list_width`。分割条悬停时高亮。默认列是方法、路径、状态、间隔、主机，按到达时间从上到下排，最新在最下面。右键一行会先选中它，菜单里可以复制 URL、cURL、只看、屏蔽、星标、基准、重放，以及填入一条 `break` 规则。方法列始终显示。表头右侧列图标和表头右键可开关状态、协议、路径、间隔、主机、大小；`show_status`、`show_path`、`show_gap`、`show_host` 默认开，`show_protocol`、`show_size` 默认关，缺这些键就保持默认。间隔列是距到达顺序里上一条的间隔，首条 `+0 ms`，被过滤掉的行仍计入间隔；悬停显示本地 `HH:MM:SS.mmm`，详情摘要用同一绝对时间。路径可见时占剩余宽度，藏起后空宽不挪给别的列。路径和主机超出时中间省略，悬停显示全文。协议默认只在详情摘要的徽章里。离底部不超过一行时列表跟着新流量，再往上翻就停住。过滤框常驻，是 AND 词：方法精确、状态精确或 `1xx`–`5xx`、`失败`、`type:` / `ct:`、`body:`、`header:`、`pin:`、`mute:` 或 `-词`，其余匹配主机或路径。`body:` 搜请求和响应 body，`header:` 搜头名和头值。`pin:` 只留星标。`slow:` 只留耗时不少于 1 秒的流量，和警告色同一条线。过滤框下的胶囊开关 `type:json`、`失败`、`pin:`、`slow:`。过滤不保存。`↑` / `↓` 在筛选后的列表里换选中行，输入框有焦点时仍移动光标。`Cmd/Ctrl+K` 清空会话，`Cmd/Ctrl+F` 聚焦过滤框，`Cmd/Ctrl+R` 重放选中行；`make dev` 可能吃掉 `Cmd+R`。直通名单、规则、上游在过滤框旁的配置图标弹出层里，写进 `config.toml` 并立刻生效，不刷新会话代际。右栏在摘要之后是查询参数、行级对比、消息、响应 body、响应头、请求头、请求 body。查询、对比、消息、两张 body 和两张头卡片点标题折叠，摘要不折叠，折叠只在这次窗口里记住。没有状态码的失败行状态是 `ERR`，路径显示记录时的简短原因；进行中是 `—`。图片在美化时按比例画在响应 body 卡片里，不裁切铺满，这时不显示十六进制截断说明；原文显示十六进制，每行带四位行号；复制不带行号。JSON、HTML、XML 和不少于 80 字节的多行文本在只读 `editor` 里显示。「编辑重发」的头和 body 用可编辑的 `editor`。行级对比用 `diff_view`。美化 / 原文在 body 卡片标题上，菜单里仍可切换。请求 body 为空且没有截断或解压说明时不画那张卡片。响应 body 没有正文、没有说明、也没有图片预览时同样不画。图片预览和证书二维码用 `image`。没有系统文件选择框。「编辑重发」可改方法、URL、头和 body；URL 框粘贴以 `curl` 开头的命令会填进这些框。停在响应断点时，「编辑重发」改状态码、响应头和响应 body。切换选中行或清空会收起编辑框。方法、状态、协议那一行的右侧「更多」收纳复制 URL、cURL、`fetch`、Python `requests`、浅层 TypeScript `interface`、请求、响应、另存、只看、屏蔽、星标、基准、重放、编辑重发和美化。断点条上的「放行」「丢弃」仍留在原处。超过 1 秒的时间列用警告色，主机和路径保持正常颜色。星标在方法徽章前有蓝点。JWT、时间戳和可打印 Base64 附在原值下面。美化时 `Cookie` 和 `Set-Cookie` 按 `;` 拆成一行一个键，原文和复制仍是整段。美化时请求头和响应头按名字忽略大小写升序，原文、cURL 和复制保持抓到的顺序和大小写。
- 证书已安装时「更多」里只显示「移除证书」，未安装时只显示「安装证书」。证书状态是菜单里的文字，不是按钮。
- 界面配色用 `AdaptiveColor::rgb` 做深浅双模式分层（画布、顶/底栏、列表、列头、斑马纹行、选中行、详情卡片、代码框），不要直接拿 `nsh_base::theme()` 默认中性主题里同色的 `secondary`/`muted`/`accent` 或纯黑背景硬铺。
- 方法（`GET`、`POST`、`PUT`/`PATCH`、`DELETE`、`CONNECT`）与状态码（`2xx`、`3xx`、`4xx`/`5xx`/失败）用语义色区分；主操作按钮（「开始」蓝底、「停止」红底）始终用亮白字。工具栏常驻「更多」和「开始」，「更多」左边是会话条数：没有过滤词时是总数，有过滤词时是「筛选数 / 总数」。「更多」里「清空」在第一项，右侧是同一数字，「导出」在第二项。「复制代理环境」、系统代理和证书也在这个菜单里。「清空」清的是整个会话。「系统代理」带状态圆点，开启时菜单项和「更多」按钮用高亮描边。工具栏、菜单、配置和空态用小的单色描边图标。
- 界面语言默认跟随系统。`config.toml` 的 `language` 写 `zh` 或 `en`，缺这一项就跟随系统。「更多」里可以改成跟随系统、中文或 English，切换后马上生效。抓包记录里的错误原文仍是写入时的语言，窗口只翻译一批固定的原因。手机安装页在 `Accept-Language` 首段不是 `zh` 时用英文，空的或 `zh` 仍是中文。
- 左栏方法用彩色徽章，状态、路径、间隔、主机、协议和大小用等宽字体；按钮和句子不用等宽。右栏摘要、请求头、请求 body、响应头、响应 body 各自收纳在圆角卡片里，URL 放在独立代码框中，请求头/响应头标题旁带数量角标，行间只用 `index > 0` 的上边框分隔，避免末行与卡片底部出现双横线。
- 协议跟随：用客户端 ClientHello 里的 ALPN 去连源站，两边都用源站选定的协议。不降级成 HTTP/1.1，不合并 Cookie，不改业务头的大小写。
- 不用 hudsucker。CONNECT 用 hyper、rustls、tokio。
- TLS 探测失败要关掉那条连接，新开 TCP，再放行缓冲的 ClientHello。探测失败的同一条 TCP 不能复用。
- ServerHello 之前失败就改成 TCP 隧道，只记目标和原因。ServerHello 之后的解析失败只记错误，不能退回隧道。
- body 是旁路拷贝。转发不等待记录和界面。先按 `Content-Encoding` 的反序解压再记录。单条解压后最多 1 MB（`BODY_CAP`），会话最多 256 MB（`SESSION_CAP`），超出丢掉最旧的未星标 flow，正在写的那条除外。星标最多 32 条（`PIN_CAP`），不参与淘汰。
- WebSocket 从旁路解析帧，转发不等解析。转发握手时去掉 `Sec-WebSocket-Extensions`，避免谈成 `permessage-deflate`。非法帧或 `RSV1` 只停止解析。消息最多 200 条或 256 KB 文本，不计入 `BODY_CAP` / `SESSION_CAP`。`Content-Type` 含 `event-stream` 且没有 WebSocket 消息时，按 SSE 事件列出。
- 清空会话会清列表、字节和丢弃计数，不清下一条 id。复制 URL、cURL、请求和响应走记录原文。JSON、表单、XML、HTML 默认美化显示，菜单可切回原文。`default_pretty` 记在 `config.toml`，「更多」里「默认美化」可改，缺这一项就是美化，切换后当前详情马上重画。卡片上的切换不写入配置。另存响应写到下载目录的 `zniff` 文件夹，状态栏给出完整路径。HAR 仍写到数据目录的 `exports/`。监听端口默认 `8877`，记在 `config.toml` 的 `listen_port`，`0` 拒绝，绑不上就报错。局域网开关是 `listen_all`，打开后绑 `0.0.0.0`。`GET /zniff` 和直接访问 `/` 返回手机安装页，`GET /zniff-ca.crt` 返回根证书。监听地址是 `0.0.0.0` 之后，底栏「扫码」点开才显示二维码，不铺在右栏。默认用访问公网的那张 IPv4；隧道和 `198.18.0.0/15` 让位给 `en` / `eth` 这类网卡。多张网卡时在弹出层里改选。指向 `http://{所选 IP}:{端口}/zniff`。
- 直通名单在 ClientHello 之后看 SNI，否则用 CONNECT 主机。空名单全部解密。`only:` 是白名单。命中后记「直通名单，未解密」并改 TCP 隧道。
- 上游写在 `config.toml` 的 `upstream_proxy`。没配时源站直连。明文 HTTP 省略 `Connection` 时补 `keep-alive`，读完且双方都没要求关闭才放回池。HTTP/1 TLS 不进池。HTTP/2 另有最多 16 条发送端缓存，同一源站的下一条客户端连接会复用。重放强制 `Connection: close`，不套 Map / 断点，不进池。编辑重发按新 body 重算 `Content-Length`，URL 改了就丢掉旧的 `Host`。
- 规则里 `map-local`、`map-remote`、`break`、`break-res` 按文本顺序第一条命中。`delay`、`header`、`cors`、`replace` 不占这条名额，匹配到的都会叠加。`replace` 只改带 `Content-Length` 且不超过 1 MB 的 UTF-8 正文。`delay` 最长 30 秒。`map-local` 可写 `status=`，省略时回 200，读不到文件记错误并回 502。`cors` 在没有终点规则的 `OPTIONS` 上直接回 204。请求断点和响应断点共用一个槽位，新断点顶替上一条并自动放行，等 60 秒，超时当放行。响应断点最多缓冲 1 MB。规则里的文件路径不能有空格。配置弹出层顶部可以临时关掉规则或只关掉断点，不改规则文本，也不写入配置。关掉任一个时配置按钮用警告色。已经停住的断点仍要放行或等超时。HAR 1.2 导出到 `exports/`。过滤框有内容时只导出匹配的流量，「导出」右侧是条数。导入在顶栏「更多」里，路径空着就读数据目录的 `import.har`，也可以把 `.har` 拖到窗口上，并还原耗时和 `bodySize`。
- 会话只在内存里。重启清空。不要把整份会话放进 nsh `Signal`。工作线程写 `Mutex`，界面大约每 16 ms 抄到 signal 上。nsh 组件只跑一遍，会变的内容用 `text(move || ...)`、`when` 和 `each`。
- 根 CA 名是 `zniff CA`。macOS 移除时同时删旧名 `Snoop CA`。退出时不删除 CA。数据目录是 macOS `~/Library/Application Support/zniff`，Windows `%APPDATA%\zniff`，Linux `$XDG_DATA_HOME/zniff` 或 `~/.local/share/zniff`。
- 系统代理默认关。「更多」里的「系统代理」打开后，按「开始」或在抓包中打开开关，才会改系统代理。关掉开关或停止时，只有这次确实改过才恢复。启动本身不打开系统代理。上次若在改过代理后异常退出，下次启动的 `restore_if_dirty()` 只把原来的设置恢复回去。源站连接直连，不走系统代理。开关记在 `config.toml` 的 `use_system_proxy`。恢复现场单独写在 `proxy-snapshot.toml`，不和用户选项放在同一个文件里。旧的分散文件和 `proxy-snapshot.json` 在第一次读取时并进这两份 TOML，然后删掉。

## 命令

在仓库根目录：

```bash
make test
make build
make dev
```

`make dev` 会打开窗口。测试绑 `127.0.0.1:0`，不要在测试里调用 `networksetup`、`security` 或改系统代理。

打开窗口会先跑 `restore_if_dirty()`。如果 `proxy-snapshot.toml` 的 `active` 是 true，这次启动会把系统代理改回去。只有旧的 `proxy-snapshot.json` 时会先转成 TOML。不要在用户正在抓包时再启动一个实例。没有要求验证抓包时，不要按「开始」，结束前确认没有留下 `zniff` 进程，并核对系统代理仍是原来的状态。改界面时要跑 `make dev` 并对窗口截图（覆盖空态、有流量选中态及深浅模式，可在内存里临时注入样例流量，不要启动监听或改系统代理），看图核对对齐、对比度和边框层次，确认后再删掉临时截图钩子。
