import Foundation

@MainActor
final class AppModel: ObservableObject {
    @Published var servers: [ServerProfile] = []
    @Published var selectedServer: ServerProfile?
    @Published var projects: [RemoteProject] = []
    @Published var sessions: [RemoteSession] = []
    @Published var selectedProject: RemoteProject?
    @Published var loading = false
    @Published var error: String?

    let acp = ACPClient()

    private let serversKey = "dunlin.servers"

    init() {
        if let data = UserDefaults.standard.data(forKey: serversKey),
           let decoded = try? JSONDecoder().decode([ServerProfile].self, from: data) {
            servers = decoded
            selectedServer = decoded.first
        }
    }

    var deviceID: String? {
        selectedServer?.deviceID
    }

    func pair(url: String, code: String, name: String) async {
        loading = true
        defer { loading = false }
        do {
            let response = try await RelayClient(baseURL: url).pair(code: code, deviceName: name)
            try Keychain.save(response.token, account: response.device_id)
            let profile = ServerProfile(
                id: response.device_id,
                name: name.isEmpty ? url : name,
                url: url,
                userName: response.user.name,
                deviceID: response.device_id
            )
            servers.append(profile)
            selectedServer = profile
            saveServers()
            await refreshProjects()
        } catch {
            self.error = error.localizedDescription
        }
    }

    func select(server: ServerProfile) async {
        selectedServer = server
        projects = []
        sessions = []
        await refreshProjects()
    }

    func refreshProjects() async {
        guard let server = selectedServer,
              let token = Keychain.load(account: server.deviceID) else { return }
        loading = true
        defer { loading = false }
        do {
            projects = try await RelayClient(baseURL: server.url, token: token).projects()
        } catch {
            self.error = error.localizedDescription
        }
    }

    func refreshSessions(project: RemoteProject) async {
        guard let server = selectedServer,
              let token = Keychain.load(account: server.deviceID) else { return }
        do {
            sessions = try await RelayClient(baseURL: server.url, token: token)
                .sessions(projectID: project.id)
        } catch {
            self.error = error.localizedDescription
        }
    }

    func open(project: RemoteProject, session: RemoteSession?) async {
        guard let server = selectedServer,
              let token = Keychain.load(account: server.deviceID) else { return }
        selectedProject = project
        loading = true
        defer { loading = false }
        do {
            try await acp.connect(
                baseURL: server.url,
                token: token,
                projectID: project.id,
                cwd: project.path
            )
            if let session {
                try await acp.loadSession(session.id)
            } else {
                try await acp.newSession()
                await refreshSessions(project: project)
            }
        } catch {
            self.error = error.localizedDescription
        }
    }

    func newSession(project: RemoteProject) async {
        await open(project: project, session: nil)
    }

    func takeover(project: RemoteProject, session: RemoteSession) async {
        guard let server = selectedServer,
              let token = Keychain.load(account: server.deviceID) else { return }
        loading = true
        defer { loading = false }
        do {
            let client = RelayClient(baseURL: server.url, token: token)
            try await client.takeover(sessionID: session.id)
            try await acp.connect(
                baseURL: server.url,
                token: token,
                projectID: project.id,
                cwd: project.path
            )
            try await acp.loadSession(session.id)
            await refreshSessions(project: project)
        } catch {
            self.error = error.localizedDescription
        }
    }

    func send(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        acp.prompt(trimmed)
    }

    func cancel() {
        acp.cancel()
    }

    func disconnect() {
        acp.disconnect()
    }

    func remove(server: ServerProfile) {
        Keychain.delete(account: server.deviceID)
        servers.removeAll { $0.id == server.id }
        if selectedServer?.id == server.id {
            selectedServer = servers.first
            projects = []
            sessions = []
        }
        saveServers()
    }

    func isActiveElsewhere(_ session: RemoteSession) -> Bool {
        guard let active = session.active_device_id else { return false }
        return active != deviceID
    }

    private func saveServers() {
        if let data = try? JSONEncoder().encode(servers) {
            UserDefaults.standard.set(data, forKey: serversKey)
        }
    }
}
