import Darwin
import Foundation
import Network
import NetworkExtension

@_silgen_name("proc_pidpath")
func zniffProcPidPath(_ pid: Int32, _ buffer: UnsafeMutableRawPointer?, _ buffersize: UInt32) -> Int32

private let selfPid = getpid()

private let hardExcludedSigningIds: Set<String> = [
    "dev.zniff.app",
    "dev.zniff.app.transparent-proxy",
    "com.apple.trustd",
    "com.apple.ocspd",
    "com.apple.mDNSResponder",
    "com.apple.nesessionmanager",
    "com.apple.neagent",
    "com.apple.sysextd",
]

private final class ProxyConfigStore {
    private let lock = NSLock()
    private var proxyPort: UInt16 = 8877
    private var proxyAuth: String = ""
    private var captureAll: Bool = false
    private var targets: [String] = []
    private var blockQuic: Bool = true

    struct Snapshot {
        let proxyPort: UInt16
        let proxyAuth: String
        let captureAll: Bool
        let targets: [String]
        let blockQuic: Bool
    }

    func snapshot() -> Snapshot {
        lock.lock()
        defer { lock.unlock() }
        return Snapshot(
            proxyPort: proxyPort,
            proxyAuth: proxyAuth,
            captureAll: captureAll,
            targets: targets,
            blockQuic: blockQuic
        )
    }

    @discardableResult
    func update(from dict: [String: Any]) -> (changedQuic: Bool, snapshot: Snapshot) {
        lock.lock()
        defer { lock.unlock() }
        let oldBlockQuic = blockQuic
        if let port = dict["proxyPort"] as? NSNumber, port.intValue > 0, port.intValue <= 65535 {
            proxyPort = UInt16(port.intValue)
        }
        if let auth = dict["proxyAuth"] as? String {
            proxyAuth = auth
        }
        if let all = dict["captureAll"] as? Bool {
            captureAll = all
        }
        if let list = dict["targets"] as? [String] {
            targets = list.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        }
        if let quic = dict["blockQuic"] as? Bool {
            blockQuic = quic
        }
        let snap = Snapshot(
            proxyPort: proxyPort,
            proxyAuth: proxyAuth,
            captureAll: captureAll,
            targets: targets,
            blockQuic: blockQuic
        )
        return (oldBlockQuic != blockQuic, snap)
    }
}

private func extractSourceProcessInfo(from metaData: NEFlowMetaData?) -> (pid: Int32?, signingId: String, execName: String, appBundleName: String?) {
    let signingId = metaData?.sourceAppSigningIdentifier ?? ""
    var pid: Int32?
    if let token = metaData?.sourceAppAuditToken, token.count >= 24 {
        let rawPid = token.withUnsafeBytes { raw -> Int32 in
            raw.load(fromByteOffset: 20, as: Int32.self)
        }
        if rawPid > 0 {
            pid = rawPid
        }
    }
    var execName = ""
    var appBundleName: String?
    if let p = pid {
        var buf = [CChar](repeating: 0, count: Int(MAXPATHLEN) * 4)
        let len = zniffProcPidPath(p, &buf, UInt32(buf.count))
        if len > 0 {
            let path = String(cString: buf)
            let url = URL(fileURLWithPath: path)
            execName = url.lastPathComponent
            for component in url.pathComponents.reversed() where component.hasSuffix(".app") {
                appBundleName = String(component.dropLast(4))
                break
            }
        }
    }
    return (pid, signingId, execName, appBundleName)
}

private func isHardExcluded(pid: Int32?, signingId: String, execName: String) -> Bool {
    if let pid, pid == selfPid {
        return true
    }
    if hardExcludedSigningIds.contains(signingId) {
        return true
    }
    let lowerExec = execName.lowercased()
    if lowerExec == "zniff" || lowerExec == "znifftransparentproxy" {
        return true
    }
    return false
}

private func matchesTargets(_ snapshot: ProxyConfigStore.Snapshot, signingId: String, execName: String, appBundleName: String?) -> Bool {
    if snapshot.captureAll {
        return true
    }
    if snapshot.targets.isEmpty {
        return false
    }
    let lowerSigning = signingId.lowercased()
    let lowerExec = execName.lowercased()
    let lowerApp = (appBundleName ?? "").lowercased()
    for rawTarget in snapshot.targets {
        let t = rawTarget.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if t.isEmpty { continue }
        if !lowerSigning.isEmpty && lowerSigning == t {
            return true
        }
        if !lowerExec.isEmpty && (lowerExec == t || lowerExec.hasPrefix(t + " ")) {
            return true
        }
        if !lowerApp.isEmpty && lowerApp == t {
            return true
        }
    }
    return false
}

private func isBypassedHost(_ host: String) -> Bool {
    let clean = host.trimmingCharacters(in: CharacterSet(charactersIn: "[]")).lowercased()
    if clean.isEmpty || clean == "localhost" || clean == "::1" || clean.hasPrefix("127.") || clean.hasPrefix("169.254.") || clean.hasPrefix("fe80:") || clean.hasPrefix("ff") {
        return true
    }
    if let firstDot = clean.firstIndex(of: "."), let firstOctet = Int(clean[..<firstDot]), (224...239).contains(firstOctet) {
        return true
    }
    return false
}

private func resolveRemoteHostPort(for flow: NEAppProxyTCPFlow) -> (host: String, port: UInt16)? {
    var host = flow.remoteHostname?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    var port: UInt16 = 0

    if #available(macOS 15.0, *) {
        if case let .hostPort(endpointHost, endpointPort) = flow.remoteFlowEndpoint {
            port = endpointPort.rawValue
            if host.isEmpty {
                switch endpointHost {
                case .name(let name, _):
                    host = name
                case .ipv4(let v4):
                    host = v4.debugDescription
                case .ipv6(let v6):
                    host = v6.debugDescription
                @unknown default:
                    break
                }
            }
        }
    } else if let endpoint = flow.remoteEndpoint as? NWHostEndpoint {
        port = UInt16(endpoint.port) ?? 0
        if host.isEmpty {
            host = endpoint.hostname
        }
    }

    if port == 0 || host.isEmpty {
        return nil
    }
    return (host, port)
}

private func formatAuthority(host: String, port: UInt16) -> String {
    let clean = host.trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
    if clean.contains(":") {
        return "[\(clean)]:\(port)"
    }
    return "\(clean):\(port)"
}

private func sanitizeHeaderValue(_ value: String) -> String {
    let scalars = value.unicodeScalars.filter { scalar in
        scalar.value >= 0x20 && scalar.value != 0x7f
    }
    return String(String.UnicodeScalarView(scalars)).trimmingCharacters(in: .whitespacesAndNewlines)
}

private final class TcpFlowRelay {
    let id: UInt64
    private let flow: NEAppProxyTCPFlow
    private let connection: NWConnection
    private let authority: String
    private let sourceApp: String
    private let proxyAuth: String
    private let queue: DispatchQueue
    private let onFinish: (UInt64) -> Void

    private let stateLock = NSLock()
    private var closed = false
    private var handshakeBuffer = Data()

    init(
        id: UInt64,
        flow: NEAppProxyTCPFlow,
        proxyPort: UInt16,
        proxyAuth: String,
        authority: String,
        sourceApp: String,
        queue: DispatchQueue,
        onFinish: @escaping (UInt64) -> Void
    ) {
        self.id = id
        self.flow = flow
        self.proxyAuth = proxyAuth
        self.authority = authority
        self.sourceApp = sourceApp
        self.queue = queue
        self.onFinish = onFinish

        let port = NWEndpoint.Port(rawValue: proxyPort) ?? NWEndpoint.Port(integerLiteral: 8877)
        self.connection = NWConnection(host: .ipv4(.loopback), port: port, using: .tcp)
    }

    func start() {
        connection.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            switch state {
            case .ready:
                self.sendConnectHandshake()
            case .failed(let error):
                self.shutdown(error: error)
            case .cancelled:
                self.shutdown(error: nil)
            default:
                break
            }
        }
        connection.start(queue: queue)
    }

    func cancel() {
        shutdown(error: nil)
    }

    private func sendConnectHandshake() {
        var req = "CONNECT \(authority) HTTP/1.1\r\nHost: \(authority)\r\n"
        if !sourceApp.isEmpty {
            req += "X-Zniff-Source-App: \(sanitizeHeaderValue(sourceApp))\r\n"
        }
        if !proxyAuth.isEmpty, let authData = proxyAuth.data(using: .utf8) {
            req += "Proxy-Authorization: Basic \(authData.base64EncodedString())\r\n"
        }
        req += "\r\n"
        guard let reqData = req.data(using: .utf8) else {
            shutdown(error: nil)
            return
        }
        connection.send(content: reqData, completion: .contentProcessed { [weak self] error in
            guard let self else { return }
            if let error {
                self.shutdown(error: error)
                return
            }
            self.readConnectResponse()
        })
    }

    private func readConnectResponse() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 4096) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            if let error {
                self.shutdown(error: error)
                return
            }
            guard let data, !data.isEmpty else {
                self.shutdown(error: nil)
                return
            }
            self.handshakeBuffer.append(data)
            let delimiter = Data([0x0d, 0x0a, 0x0d, 0x0a])
            if let range = self.handshakeBuffer.range(of: delimiter) {
                let headerData = self.handshakeBuffer.subdata(in: 0..<range.lowerBound)
                let remainder = self.handshakeBuffer.subdata(in: range.upperBound..<self.handshakeBuffer.count)
                self.handshakeBuffer.removeAll(keepingCapacity: false)

                let headerText = String(data: headerData, encoding: .utf8) ?? ""
                let statusLine = headerText.components(separatedBy: "\r\n").first ?? ""
                let parts = statusLine.split(separator: " ")
                let statusOk = parts.count >= 2 && parts[1] == "200"
                guard statusOk else {
                    self.shutdown(error: nil)
                    return
                }
                self.openFlowAndPump(initialDownstream: remainder)
                return
            }
            if isComplete || self.handshakeBuffer.count > 16384 {
                self.shutdown(error: nil)
                return
            }
            self.readConnectResponse()
        }
    }

    private func openFlowAndPump(initialDownstream: Data) {
        flow.open(withLocalEndpoint: nil) { [weak self] error in
            guard let self else { return }
            if let error {
                self.shutdown(error: error)
                return
            }
            self.pumpUpstream()
            if !initialDownstream.isEmpty {
                self.flow.write(initialDownstream) { [weak self] writeErr in
                    guard let self else { return }
                    if let writeErr {
                        self.shutdown(error: writeErr)
                        return
                    }
                    self.pumpDownstream()
                }
            } else {
                self.pumpDownstream()
            }
        }
    }

    private func pumpUpstream() {
        if isClosed() { return }
        flow.readData { [weak self] data, error in
            guard let self else { return }
            if let error {
                self.shutdown(error: error)
                return
            }
            guard let data, !data.isEmpty else {
                self.connection.send(content: nil, contentContext: .finalMessage, isComplete: true, completion: .contentProcessed { _ in })
                self.flow.closeReadWithError(nil)
                return
            }
            self.connection.send(content: data, completion: .contentProcessed { [weak self] sendErr in
                guard let self else { return }
                if let sendErr {
                    self.shutdown(error: sendErr)
                    return
                }
                self.pumpUpstream()
            })
        }
    }

    private func pumpDownstream() {
        if isClosed() { return }
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            if let error {
                self.shutdown(error: error)
                return
            }
            if let data, !data.isEmpty {
                self.flow.write(data) { [weak self] writeErr in
                    guard let self else { return }
                    if let writeErr {
                        self.shutdown(error: writeErr)
                        return
                    }
                    if isComplete {
                        self.flow.closeWriteWithError(nil)
                        self.shutdown(error: nil)
                    } else {
                        self.pumpDownstream()
                    }
                }
            } else {
                self.flow.closeWriteWithError(nil)
                self.shutdown(error: nil)
            }
        }
    }

    private func isClosed() -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        return closed
    }

    private func shutdown(error: Error?) {
        stateLock.lock()
        if closed {
            stateLock.unlock()
            return
        }
        closed = true
        stateLock.unlock()

        flow.closeReadWithError(error)
        flow.closeWriteWithError(error)
        connection.cancel()
        onFinish(id)
    }
}

final class TransparentProxyProvider: NETransparentProxyProvider {
    private let configStore = ProxyConfigStore()
    private let relayQueue = DispatchQueue(label: "dev.zniff.app.transparent-proxy.relay", attributes: .concurrent)
    private let relaysLock = NSLock()
    private var relays: [UInt64: TcpFlowRelay] = [:]
    private var nextRelayId: UInt64 = 1
    private var interceptedFlows: UInt64 = 0

    override func startProxy(options: [String: Any]? = nil, completionHandler: @escaping (Error?) -> Void) {
        if let proto = protocolConfiguration as? NETunnelProviderProtocol,
           let providerConfig = proto.providerConfiguration {
            _ = configStore.update(from: providerConfig)
        }
        let snapshot = configStore.snapshot()
        applyNetworkSettings(blockQuic: snapshot.blockQuic, completionHandler: completionHandler)
    }

    override func stopProxy(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        relaysLock.lock()
        let active = Array(relays.values)
        relays.removeAll()
        relaysLock.unlock()
        for relay in active {
            relay.cancel()
        }
        completionHandler()
    }

    override func handleAppMessage(_ messageData: Data, completionHandler: ((Data?) -> Void)?) {
        var changedQuic = false
        var blockQuic = configStore.snapshot().blockQuic
        if let json = try? JSONSerialization.jsonObject(with: messageData) as? [String: Any] {
            let res = configStore.update(from: json)
            changedQuic = res.changedQuic
            blockQuic = res.snapshot.blockQuic
        }
        let reply = {
            self.relaysLock.lock()
            let activeCount = self.relays.count
            let totalCount = self.interceptedFlows
            self.relaysLock.unlock()
            let payload: [String: Any] = [
                "ok": true,
                "activeFlows": activeCount,
                "interceptedFlows": totalCount,
            ]
            completionHandler?(try? JSONSerialization.data(withJSONObject: payload))
        }
        if changedQuic {
            applyNetworkSettings(blockQuic: blockQuic) { _ in
                reply()
            }
        } else {
            reply()
        }
    }

    override func handleNewFlow(_ flow: NEAppProxyFlow) -> Bool {
        let info = extractSourceProcessInfo(from: flow.metaData)
        if isHardExcluded(pid: info.pid, signingId: info.signingId, execName: info.execName) {
            return false
        }
        let snapshot = configStore.snapshot()
        guard matchesTargets(snapshot, signingId: info.signingId, execName: info.execName, appBundleName: info.appBundleName) else {
            return false
        }

        if let udpFlow = flow as? NEAppProxyUDPFlow {
            guard snapshot.blockQuic else {
                return false
            }
            udpFlow.open(withLocalEndpoint: nil) { _ in
                udpFlow.closeReadWithError(nil)
                udpFlow.closeWriteWithError(nil)
            }
            return true
        }

        guard let tcpFlow = flow as? NEAppProxyTCPFlow,
              let (host, port) = resolveRemoteHostPort(for: tcpFlow) else {
            return false
        }
        if port == 53 || isBypassedHost(host) {
            return false
        }

        let authority = formatAuthority(host: host, port: port)
        let sourceApp = info.appBundleName ?? (!info.execName.isEmpty ? info.execName : info.signingId)

        relaysLock.lock()
        let relayId = nextRelayId
        nextRelayId += 1
        interceptedFlows += 1
        let relay = TcpFlowRelay(
            id: relayId,
            flow: tcpFlow,
            proxyPort: snapshot.proxyPort,
            proxyAuth: snapshot.proxyAuth,
            authority: authority,
            sourceApp: sourceApp,
            queue: relayQueue
        ) { [weak self] finishedId in
            guard let self else { return }
            self.relaysLock.lock()
            self.relays.removeValue(forKey: finishedId)
            self.relaysLock.unlock()
        }
        relays[relayId] = relay
        relaysLock.unlock()

        relay.start()
        return true
    }

    private func applyNetworkSettings(blockQuic: Bool, completionHandler: @escaping (Error?) -> Void) {
        let settings = NETransparentProxyNetworkSettings(tunnelRemoteAddress: "127.0.0.1")
        var included: [NENetworkRule] = [
            NENetworkRule(
                remoteNetwork: nil,
                remotePrefix: 0,
                localNetwork: nil,
                localPrefix: 0,
                protocol: .TCP,
                direction: .outbound
            )
        ]
        if blockQuic {
            included.append(
                NENetworkRule(
                    remoteNetwork: NWHostEndpoint(hostname: "0.0.0.0", port: "443"),
                    remotePrefix: 0,
                    localNetwork: nil,
                    localPrefix: 0,
                    protocol: .UDP,
                    direction: .outbound
                )
            )
            included.append(
                NENetworkRule(
                    remoteNetwork: NWHostEndpoint(hostname: "::", port: "443"),
                    remotePrefix: 0,
                    localNetwork: nil,
                    localPrefix: 0,
                    protocol: .UDP,
                    direction: .outbound
                )
            )
        }
        settings.includedNetworkRules = included
        settings.excludedNetworkRules = [
            NENetworkRule(
                remoteNetwork: NWHostEndpoint(hostname: "127.0.0.0", port: "0"),
                remotePrefix: 8,
                localNetwork: nil,
                localPrefix: 0,
                protocol: .any,
                direction: .outbound
            ),
            NENetworkRule(
                remoteNetwork: NWHostEndpoint(hostname: "::1", port: "0"),
                remotePrefix: 128,
                localNetwork: nil,
                localPrefix: 0,
                protocol: .any,
                direction: .outbound
            ),
        ]
        setTunnelNetworkSettings(settings, completionHandler: completionHandler)
    }
}

autoreleasepool {
    NEProvider.startSystemExtensionMode()
}
dispatchMain()
