import SwiftUI

struct PairingView: View {
    @EnvironmentObject private var model: AppModel
    @State private var url = "http://127.0.0.1:8787"
    @State private var code = ""
    @State private var name = "My iPhone"

    var body: some View {
        Form {
            Section("Relay") {
                TextField("Relay URL", text: $url)
                    .textInputAutocapitalization(.never)
                    .keyboardType(.URL)
                    .autocorrectionDisabled()
                Text("Use the Tailscale hostname for a physical device, or 127.0.0.1 in the simulator.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Pairing code") {
                TextField("ABC12345", text: $code)
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
                Text("Run `dunlin-relay pair <user>` on the server.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("This device") {
                TextField("Device name", text: $name)
            }
            Section {
                Button {
                    Task { await model.pair(url: url, code: code, name: name) }
                } label: {
                    HStack {
                        Spacer()
                        if model.loading {
                            ProgressView()
                        } else {
                            Text("Pair device").fontWeight(.semibold)
                        }
                        Spacer()
                    }
                }
                .disabled(model.loading || code.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .navigationTitle("Connect to Dunlin")
    }
}

struct ServersView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        List {
            Section("Servers") {
                ForEach(model.servers) { server in
                    Button {
                        Task { await model.select(server: server) }
                    } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(server.name)
                                Text("\(server.url) · \(server.userName)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            if model.selectedServer?.id == server.id {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                    .swipeActions {
                        Button(role: .destructive) {
                            model.remove(server: server)
                        } label: {
                            Label("Remove", systemImage: "trash")
                        }
                    }
                }
            }
            Section {
                NavigationLink("Pair another server") {
                    PairingView()
                }
            }
        }
        .navigationTitle("Servers")
    }
}
