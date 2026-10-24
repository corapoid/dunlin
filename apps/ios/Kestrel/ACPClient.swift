import Foundation

@MainActor
final class ACPClient: ObservableObject {
    @Published private(set) var state: ConnectionState = .offline
    @Published private(set) var messages: [ChatItem] = []
    @Published var permission: PermissionRequest?
    @Published private(set) var sessionID: String?

    private var task: URLSessionWebSocketTask?
    private var nextID = 1
    private var pending: [RPCID: CheckedContinuation<[String: Any], Error>] = [:]
    private var streamingAgent: Int?
    private var streamingThought: Int?
    private var pingTask: Task<Void, Never>?
    private var cwd: String = ""

    func connect(baseURL: String, token: String, projectID: String, cwd: String) async throws {
        disconnect()
        guard var components = URLComponents(string: baseURL.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            throw RelayError.badURL
        }
        components.scheme = components.scheme == "https" ? "wss" : "ws"
        let prefix = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.path = prefix.isEmpty ? "/acp" : "/\(prefix)/acp"
        components.queryItems = [URLQueryItem(name: "project", value: projectID)]
        guard let url = components.url else { throw RelayError.badURL }

        var request = URLRequest(url: url)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let socket = URLSession.shared.webSocketTask(with: request)
        task = socket
        self.cwd = cwd
        state = .connecting
        socket.resume()
        listen()
        _ = try await sendRequest("initialize", params: [
            "protocolVersion": 1,
            "clientCapabilities": [:],
        ])
        state = .ready
        startPing()
    }

    func newSession() async throws {
        let result = try await sendRequest("session/new", params: [
            "cwd": cwd,
            "mcpServers": [],
        ])
        sessionID = result["sessionId"] as? String
    }

    func loadSession(_ id: String) async throws {
        messages.removeAll()
        streamingAgent = nil
        streamingThought = nil
        _ = try await sendRequest("session/load", params: [
            "sessionId": id,
            "cwd": cwd,
            "mcpServers": [],
        ])
        sessionID = id
    }

    func prompt(_ text: String) {
        guard let sessionID else { return }
        messages.append(ChatItem(kind: .user(text)))
        streamingAgent = nil
        streamingThought = nil
        Task {
            _ = try? await sendRequest("session/prompt", params: [
                "sessionId": sessionID,
                "prompt": [["type": "text", "text": text]],
            ])
        }
    }

    func cancel() {
        guard let sessionID else { return }
        sendNotification("session/cancel", params: ["sessionId": sessionID])
    }

    func respondPermission(_ optionID: String?) {
        guard let permission else { return }
        let result: [String: Any] = optionID.map {
            ["outcome": ["outcome": "selected", "optionId": $0]]
        } ?? ["outcome": ["outcome": "cancelled"]]
        sendResponse(id: permission.rpcID, result: result)
        self.permission = nil
    }

    func disconnect() {
        pingTask?.cancel()
        pingTask = nil
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        state = .offline
        let waiting = pending
        pending.removeAll()
        for (_, continuation) in waiting {
            continuation.resume(throwing: RelayError.transport("Disconnected"))
        }
    }

    private func listen() {
        task?.receive { [weak self] result in
            Task { @MainActor in
                guard let self else { return }
                switch result {
                case .success(let message):
                    if case .string(let text) = message {
                        self.handle(text)
                    }
                    self.listen()
                case .failure(let error):
                    self.state = .failed(error.localizedDescription)
                }
            }
        }
    }

    private func startPing() {
        pingTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(20))
                self?.ping()
            }
        }
    }

    private func ping() {
        task?.sendPing { _ in }
    }

    private func handle(_ text: String) {
        guard let data = text.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return
        }
        if let method = object["method"] as? String {
            if let idValue = object["id"], let id = RPCID(idValue) {
                if method == "session/request_permission" {
                    handlePermission(id: id, params: object["params"] as? [String: Any] ?? [:])
                } else {
                    sendResponse(id: id, result: ["outcome": ["outcome": "cancelled"]])
                }
            } else {
                handleNotification(method: method, params: object["params"] as? [String: Any] ?? [:])
            }
            return
        }
        if let idValue = object["id"], let id = RPCID(idValue) {
            guard let continuation = pending.removeValue(forKey: id) else { return }
            if let error = object["error"] as? [String: Any] {
                continuation.resume(throwing: RelayError.protocolError(error["message"] as? String ?? "ACP error"))
            } else {
                continuation.resume(returning: object["result"] as? [String: Any] ?? [:])
            }
        }
    }

    private func handleNotification(method: String, params: [String: Any]) {
        guard method == "session/update",
              let update = params["update"] as? [String: Any],
              let kind = update["sessionUpdate"] as? String else {
            return
        }
        switch kind {
        case "agent_message_chunk":
            if let text = contentText(update["content"]) {
                appendAgent(text)
            }
        case "agent_thought_chunk":
            if let text = contentText(update["content"]) {
                appendThought(text)
            }
        case "tool_call":
            upsertTool(update, delta: false)
        case "tool_call_update":
            upsertTool(update, delta: true)
        case "plan":
            let entries = update["entries"] as? [[String: Any]] ?? []
            let steps = entries.enumerated().map { index, entry in
                PlanStep(
                    id: "\(index)",
                    content: entry["content"] as? String ?? "Step",
                    status: entry["status"] as? String ?? "pending"
                )
            }
            if let last = messages.last, case .plan = last.kind {
                messages[messages.count - 1] = ChatItem(kind: .plan(steps))
            } else {
                messages.append(ChatItem(kind: .plan(steps)))
            }
        default:
            break
        }
    }

    private func handlePermission(id: RPCID, params: [String: Any]) {
        let toolCall = params["toolCall"] as? [String: Any] ?? [:]
        let title = toolCall["title"] as? String ?? "Tool call"
        let options = (params["options"] as? [[String: Any]] ?? []).compactMap { option -> PermissionOption? in
            guard let optionID = option["optionId"] as? String else { return nil }
            return PermissionOption(
                id: optionID,
                label: option["name"] as? String ?? optionID,
                kind: option["kind"] as? String ?? "unknown"
            )
        }
        permission = PermissionRequest(
            id: id.stringValue,
            rpcID: id,
            title: title,
            options: options
        )
    }

    private func appendAgent(_ text: String) {
        if let index = streamingAgent, index < messages.count,
           case .agent(let current) = messages[index].kind {
            messages[index] = ChatItem(kind: .agent(current + text))
        } else {
            messages.append(ChatItem(kind: .agent(text)))
            streamingAgent = messages.count - 1
        }
        streamingThought = nil
    }

    private func appendThought(_ text: String) {
        if let index = streamingThought, index < messages.count,
           case .thought(let current) = messages[index].kind {
            messages[index] = ChatItem(kind: .thought(current + text))
        } else {
            messages.append(ChatItem(kind: .thought(text)))
            streamingThought = messages.count - 1
        }
    }

    private func upsertTool(_ update: [String: Any], delta: Bool) {
        let id = update["toolCallId"] as? String ?? "tool"
        let title = update["title"] as? String
        let status = update["status"] as? String
        let detail = toolDetail(update)
        if let index = messages.firstIndex(where: {
            if case .tool(let card) = $0.kind { return card.id == id }
            return false
        }), case .tool(var card) = messages[index].kind {
            if let title { card.title = title }
            if let status { card.status = status }
            if let detail, !detail.isEmpty { card.detail = detail }
            messages[index] = ChatItem(kind: .tool(card))
        } else if !delta {
            messages.append(ChatItem(kind: .tool(ToolCard(
                id: id,
                title: title ?? "Tool call",
                status: status ?? "pending",
                detail: detail ?? ""
            ))))
        }
    }

    private func toolDetail(_ update: [String: Any]) -> String? {
        guard let content = update["content"] as? [[String: Any]] else { return nil }
        for block in content {
            if let text = block["text"] as? String { return text }
            if let path = block["path"] as? String { return path }
            if let inner = block["content"] as? [String: Any], let text = inner["text"] as? String {
                return text
            }
        }
        return nil
    }

    private func contentText(_ content: Any?) -> String? {
        if let content = content as? [String: Any] {
            return content["text"] as? String
        }
        return nil
    }

    private func sendRequest(_ method: String, params: [String: Any]) async throws -> [String: Any] {
        guard task != nil else { throw RelayError.transport("Not connected") }
        let id = RPCID.int(nextID)
        nextID += 1
        return try await withCheckedThrowingContinuation { continuation in
            pending[id] = continuation
            send([
                "jsonrpc": "2.0",
                "id": id.json,
                "method": method,
                "params": params,
            ])
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(45))
                self?.timeout(id)
            }
        }
    }

    private func timeout(_ id: RPCID) {
        guard let continuation = pending.removeValue(forKey: id) else { return }
        continuation.resume(throwing: RelayError.transport("ACP request timed out"))
    }

    private func sendNotification(_ method: String, params: [String: Any]) {
        send(["jsonrpc": "2.0", "method": method, "params": params])
    }

    private func sendResponse(id: RPCID, result: [String: Any]) {
        send(["jsonrpc": "2.0", "id": id.json, "result": result])
    }

    private func send(_ object: [String: Any]) {
        guard let task,
              let data = try? JSONSerialization.data(withJSONObject: object),
              let text = String(data: data, encoding: .utf8) else {
            return
        }
        task.send(.string(text)) { [weak self] error in
            guard let error else { return }
            Task { @MainActor in
                self?.state = .failed(error.localizedDescription)
            }
        }
    }
}

extension RPCID {
    var stringValue: String {
        switch self {
        case .int(let value): return "int:\(value)"
        case .string(let value): return "string:\(value)"
        }
    }
}
