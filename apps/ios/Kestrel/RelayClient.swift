import Foundation

struct RelayClient {
    let baseURL: String
    var token: String?

    func pair(code: String, deviceName: String) async throws -> PairResponse {
        try await post("/v1/pair", body: [
            "code": code.trimmingCharacters(in: .whitespacesAndNewlines).uppercased(),
            "device_name": deviceName,
        ])
    }

    func projects() async throws -> [RemoteProject] {
        try await get("/v1/projects")
    }

    func sessions(projectID: String) async throws -> [RemoteSession] {
        try await get("/v1/sessions?project=\(encode(projectID))")
    }

    func takeover(sessionID: String) async throws {
        _ = try await send(
            path: "/v1/sessions/\(encode(sessionID))/takeover",
            method: "POST",
            body: [:]
        )
    }

    func deleteSession(id: String) async throws {
        _ = try await send(path: "/v1/sessions/\(encode(id))", method: "DELETE", body: nil)
    }

    private func get<T: Decodable>(_ path: String) async throws -> T {
        let data = try await send(path: path, method: "GET", body: nil)
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func post<T: Decodable>(_ path: String, body: [String: Any]) async throws -> T {
        let data = try await send(path: path, method: "POST", body: body)
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func send(path: String, method: String, body: [String: Any]?) async throws -> Data {
        guard var components = URLComponents(string: baseURL.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            throw RelayError.badURL
        }
        let basePath = components.path
        components.path = basePath.trimmingCharacters(in: CharacterSet(charactersIn: "/")) .isEmpty
            ? path
            : basePath.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + path
        guard let url = components.url else { throw RelayError.badURL }

        var request = URLRequest(url: url)
        request.httpMethod = method
        request.timeoutInterval = 20
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        if let body {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }

        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw RelayError.transport("The relay returned no HTTP response")
            }
            guard (200..<300).contains(http.statusCode) else {
                let message = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String
                    ?? String(data: data, encoding: .utf8)
                    ?? "Request failed"
                throw RelayError.http(http.statusCode, message)
            }
            return data
        } catch let error as RelayError {
            throw error
        } catch {
            throw RelayError.transport(error.localizedDescription)
        }
    }

    private func encode(_ value: String) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~/")
        return value.addingPercentEncoding(withAllowedCharacters: allowed) ?? value
    }
}
