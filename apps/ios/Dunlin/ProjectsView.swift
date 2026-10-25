import SwiftUI

struct ProjectsView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        List {
            if let server = model.selectedServer {
                Section("Server") {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(server.name)
                        Text(server.url)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Section("Projects") {
                if model.projects.isEmpty && !model.loading {
                    ContentUnavailableView(
                        "No projects",
                        systemImage: "folder",
                        description: Text("Ask the relay admin to grant this user a project.")
                    )
                }
                ForEach(model.projects) { project in
                    NavigationLink(value: project) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(project.name)
                            Text(project.path)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .navigationTitle("Dunlin")
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                NavigationLink {
                    ServersView()
                } label: {
                    Image(systemName: "server.rack")
                }
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { await model.refreshProjects() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
            }
        }
        .navigationDestination(for: RemoteProject.self) { project in
            SessionsView(project: project)
        }
        .task { await model.refreshProjects() }
    }
}
