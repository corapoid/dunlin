import SwiftUI

struct ChatView: View {
    let project: RemoteProject
    let session: RemoteSession?
    @EnvironmentObject private var model: AppModel
    @ObservedObject var acp: ACPClient
    @State private var draft = ""
    @State private var opened = false

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        ForEach(acp.messages) { item in
                            ChatRow(item: item)
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .padding(14)
                }
                .onChange(of: acp.messages.count) {
                    withAnimation(.easeOut(duration: 0.15)) {
                        proxy.scrollTo("bottom", anchor: .bottom)
                    }
                }
            }

            Divider()

            HStack(alignment: .bottom, spacing: 8) {
                TextField("Ask Kestrel to build, fix, or explain…", text: $draft, axis: .vertical)
                    .lineLimit(1...5)
                    .textFieldStyle(.roundedBorder)
                Button {
                    model.send(draft)
                    draft = ""
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.title2)
                }
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                if acp.state == .ready {
                    Button {
                        model.cancel()
                    } label: {
                        Image(systemName: "stop.circle")
                            .font(.title2)
                    }
                }
            }
            .padding(12)
        }
        .navigationTitle(session?.title ?? "New session")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Text(acp.state.label)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
        .task {
            guard !opened else { return }
            opened = true
            await model.open(project: project, session: session)
        }
        .sheet(item: $acp.permission) { permission in
            PermissionSheet(permission: permission, acp: acp)
        }
    }
}

struct ChatRow: View {
    let item: ChatItem

    var body: some View {
        switch item.kind {
        case .user(let text):
            HStack {
                Spacer(minLength: 40)
                Text(text)
                    .padding(10)
                    .background(Color.accentColor.opacity(0.14))
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            }
        case .agent(let text):
            MarkdownText(text: text)
                .frame(maxWidth: .infinity, alignment: .leading)
        case .thought(let text):
            Label {
                Text(text)
            } icon: {
                Image(systemName: "brain")
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        case .tool(let card):
            ToolCardView(card: card)
        case .plan(let steps):
            PlanView(steps: steps)
        case .error(let message):
            Label(message, systemImage: "exclamationmark.triangle")
                .font(.callout)
                .foregroundStyle(.red)
        }
    }
}

struct MarkdownText: View {
    let text: String

    var body: some View {
        if let attributed = try? AttributedString(
            markdown: text,
            options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        ) {
            Text(attributed)
                .textSelection(.enabled)
        } else {
            Text(text)
                .textSelection(.enabled)
        }
    }
}

struct ToolCardView: View {
    let card: ToolCard

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: "wrench.and.screwdriver")
                Text(card.title)
                    .font(.subheadline.weight(.medium))
                Spacer()
                Text(card.status)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            if !card.detail.isEmpty {
                Text(card.detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(4)
            }
        }
        .padding(10)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

struct PlanView: View {
    let steps: [PlanStep]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("PLAN")
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.secondary)
            ForEach(steps) { step in
                HStack(spacing: 6) {
                    Image(systemName: icon(for: step.status))
                        .foregroundStyle(color(for: step.status))
                    Text(step.content)
                        .font(.caption)
                }
            }
        }
        .padding(10)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    private func icon(for status: String) -> String {
        switch status {
        case "completed": return "checkmark.circle.fill"
        case "in_progress": return "circle.dotted"
        default: return "circle"
        }
    }

    private func color(for status: String) -> Color {
        switch status {
        case "completed": return .green
        case "in_progress": return .blue
        default: return .secondary
        }
    }
}

struct PermissionSheet: View {
    let permission: PermissionRequest
    @ObservedObject var acp: ACPClient

    var body: some View {
        NavigationStack {
            List {
                Section("Request") {
                    Text(permission.title)
                }
                Section("Options") {
                    ForEach(permission.options) { option in
                        Button(option.label) {
                            acp.respondPermission(option.id)
                        }
                    }
                    Button("Cancel", role: .cancel) {
                        acp.respondPermission(nil)
                    }
                }
            }
            .navigationTitle("Permission")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}
