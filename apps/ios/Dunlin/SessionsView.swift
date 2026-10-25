import SwiftUI

struct SessionsView: View {
    let project: RemoteProject
    @EnvironmentObject private var model: AppModel
    @State private var showNewChat = false

    var body: some View {
        List {
            Section {
                Button {
                    showNewChat = true
                } label: {
                    Label("New session", systemImage: "plus")
                }
            }
            Section("Sessions") {
                if model.sessions.isEmpty {
                    ContentUnavailableView(
                        "No sessions",
                        systemImage: "bubble.left.and.bubble.right",
                        description: Text("Start a session here or on another device.")
                    )
                }
                ForEach(model.sessions) { session in
                    VStack(alignment: .leading, spacing: 4) {
                        NavigationLink(value: session) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(session.title)
                                Text(relative(session.updated_at))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        if model.isActiveElsewhere(session) {
                            HStack {
                                Label("Active on another device", systemImage: "iphone.gen3")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                Spacer()
                                Button("Take over") {
                                    Task {
                                        await model.takeover(project: project, session: session)
                                    }
                                }
                                .font(.caption)
                                .buttonStyle(.bordered)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle(project.name)
        .navigationDestination(for: RemoteSession.self) { session in
            ChatView(project: project, session: session, acp: model.acp)
        }
        .navigationDestination(isPresented: $showNewChat) {
            ChatView(project: project, session: nil, acp: model.acp)
        }
        .refreshable { await model.refreshSessions(project: project) }
        .task { await model.refreshSessions(project: project) }
    }

    private func relative(_ epoch: Int) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(epoch))
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}
