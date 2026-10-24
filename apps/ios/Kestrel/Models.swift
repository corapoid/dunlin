import Foundation

struct RelayUser: Codable, Hashable {
    let id: String
    let name: String
    let role: String
}

struct PairResponse: Codable {
    let token: String
    let user: RelayUser
    let device_id: String
}

struct RemoteProject: Codable, Identifiable, Hashable {
    let id: String
    let name: String
    let path: String
}

struct RemoteSession: Codable, Identifiable, Hashable {
    let id: String
    let title: String
    let updated_at: Int
    let last_device_id: String?
    let active_device_id: String?
}

struct ServerProfile: Codable, Identifiable, Hashable {
    var id: String
    var name: String
    var url: String
    var userName: String
    var deviceID: String
}

enum ConnectionState: Equatable {
    case offline
    case connecting
    case ready
    case failed(String)

    var label: String {
        switch self {
        case .offline: return "Offline"
        case .connecting: return "Connecting…"
        case .ready: return "Connected"
        case .failed(let message): return message
        }
    }
}

struct PermissionOption: Identifiable, Hashable {
    let id: String
    let label: String
    let kind: String
}

struct PermissionRequest: Identifiable, Hashable {
    let id: String
    let rpcID: RPCID
    let title: String
    let options: [PermissionOption]
}

struct ToolCard: Identifiable, Hashable {
    let id: String
    var title: String
    var status: String
    var detail: String
}

struct PlanStep: Identifiable, Hashable {
    let id: String
    var content: String
    var status: String
}

struct ChatItem: Identifiable, Hashable {
    let id = UUID()
    var kind: Kind

    enum Kind: Hashable {
        case user(String)
        case agent(String)
        case thought(String)
        case tool(ToolCard)
        case plan([PlanStep])
        case error(String)
    }
}

enum RPCID: Hashable {
    case int(Int)
    case string(String)

    init?(_ value: Any) {
        if let int = value as? Int {
            self = .int(int)
        } else if let string = value as? String {
            self = .string(string)
        } else {
            return nil
        }
    }

    var json: Any {
        switch self {
        case .int(let value): return value
        case .string(let value): return value
        }
    }
}

enum RelayError: LocalizedError {
    case badURL
    case http(Int, String)
    case transport(String)
    case protocolError(String)

    var errorDescription: String? {
        switch self {
        case .badURL: return "Invalid relay URL"
        case .http(let code, let message): return "Relay error \(code): \(message)"
        case .transport(let message): return message
        case .protocolError(let message): return message
        }
    }
}
