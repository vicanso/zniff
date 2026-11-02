# zniff 已知限制

这一刀先把抓包跑起来。下面这些是已经接受的缺口，不挡当前开发。

## 这一刀不做

- HTTP/3、QUIC。UDP 不经过 HTTP 代理。
- Firefox 的证书库。Firefox 默认也不代理 localhost。验收浏览器是 Chrome、Safari、Edge。
- 规则、断点、HAR、透明代理、按主机跳过、JSON 树、SQLite、cyberapi。
- 会话只在内存里。重启清空。单条 body 解压后最多 1 MB，整段会话最多 256 MB，超出丢掉最旧的整条 flow。
- WebSocket 只记 HTTP 握手，之后两边对拷，不解析帧。帧字节不计入 1 MB。

## 平台

- Linux 非 GNOME：不改系统代理，界面给出监听地址，需要自己指到 `127.0.0.1:端口`。
- Windows 只改当前用户的 WinINET，不改 WinHTTP。
- macOS `security add-trusted-cert` 可能要用户点确认。`security find-certificate` 只说明证书在登录钥匙串里，不保证 SSL 信任策略已经生效。
- 带密码的系统代理：`networksetup` 读不回密码，停止后可能恢复不完整。
- Windows、Linux 上「移除证书」还没接上。macOS 用 `security delete-certificate`。

## 代理行为

- 源站连接是直连 TCP，不走系统代理。没有连接池。明文 HTTP 在客户端没带 `Connection` 时补上 `Connection: close`。
- 源站 TLS 探测失败后会关掉那条探测连接，另开一条 TCP，再把缓冲的 ClientHello 放进去。探测失败的同一条 TCP 不能复用。
- 源站没选 ALPN 时按 HTTP/1.1 记。
- ServerHello 之后客户端断开，界面尽量记成「证书被拒绝」。这是根据连接被重置做的判断，不是证书钉扎的专用检测。
- HTTP/2 这一刀只表示两边用源站选定的协议。不解析帧、HPACK、trailer、gRPC。
- 界面里非 UTF-8 body 只显示前 4 KB 十六进制，UTF-8 只显示前 256 KB。记录本身仍保留最多 1 MB。
