import SwiftUI

/// A circle's chat: the history, a composer, and the relay read every three seconds while open.
@MainActor
final class ChatModel: ObservableObject {
    let core: Core
    let circle: String
    @Published private(set) var messages: [ChatMessage] = []
    @Published var draft = ""
    @Published private(set) var sending = false
    @Published var status: String?
    @Published var connected = false

    init(core: Core, circle: String) {
        self.core = core
        self.circle = circle
    }

    func load() async {
        messages = (try? await core.run { [circle] in $0.history(circle: circle) }) ?? messages
    }

    func poll() async {
        do {
            let fresh = try await core.run { [circle] in try $0.poll(circle: circle) }
            core.store.relax()
            connected = true
            if status != nil, !sending { status = nil }
            if !fresh.isEmpty || messages.contains(where: \.pending) { await load() }
        } catch {
            connected = false
            status = plain(error)
        }
    }

    func run() async {
        await load()
        while !Task.isCancelled {
            await poll()
            try? await Task.sleep(for: .seconds(3))
        }
    }

    func send() async {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !sending else { return }
        sending = true
        defer { sending = false }
        do {
            try await core.run { [circle] in try $0.sendChat(circle: circle, text: text) }
            draft = ""
            status = nil
        } catch {
            status = "Not sent: \(plain(error))"
        }
        await load()
    }

    func rotate() async -> Bool {
        do {
            try await core.run { [circle] in try $0.rotate(circle: circle) }
            return true
        } catch {
            status = plain(error)
            return false
        }
    }
}

struct ChatView: View {
    @EnvironmentObject private var app: AppModel
    @StateObject private var model: ChatModel
    @State private var showSettings = false
    @State private var showRelay = false
    @State private var notice: String?

    init(core: Core, circle: String) {
        _model = StateObject(wrappedValue: ChatModel(core: core, circle: circle))
    }

    private var summary: CircleSummary? { app.circle(model.circle) }

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { reader in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        if model.messages.isEmpty {
                            VStack(alignment: .leading, spacing: 8) {
                                Text("A little space for your people").font(.footnote).foregroundStyle(.secondary)
                                Text("Keep the conversation going.").font(.title2.weight(.semibold))
                                Text("Start with a hello. Messages are encrypted on your device before the relay delivers them.")
                                    .foregroundStyle(.secondary)
                            }
                            .padding(.top, 40)
                        }
                        ForEach(model.messages, id: \.id) { m in
                            Bubble(message: m).id(m.id)
                        }
                    }
                    .padding()
                }
                .onChange(of: model.messages.last?.id) { _, last in
                    if let last { withAnimation { reader.scrollTo(last, anchor: .bottom) } }
                }
            }
            if let notice { StatusLine(text: notice, problem: false).padding(.horizontal) }
            StatusLine(text: model.status).padding(.horizontal)
            if summary?.hasRoot == false {
                Text("This circle was made before root epochs. Run a new ceremony to chat.")
                    .font(.footnote).foregroundStyle(.secondary).padding()
            } else {
                HStack(alignment: .bottom) {
                    TextField("Message", text: $model.draft, axis: .vertical)
                        .lineLimit(1...5)
                        .textFieldStyle(.roundedBorder)
                        .accessibilityIdentifier("message")
                        .onSubmit { Task { await model.send() } }
                    Button {
                        Task { await model.send() }
                    } label: {
                        if model.sending { ProgressView() } else { Text("Send") }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(model.sending || model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityIdentifier("send")
                }
                .padding()
            }
        }
        .navigationTitle(summary?.name ?? "Chat")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 0) {
                    Text(summary?.name ?? "Chat").font(.headline)
                    Text("\(summary?.members ?? 0) people · End-to-end encrypted")
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
            ToolbarItemGroup(placement: .topBarTrailing) {
                NavigationLink(value: Route.people(model.circle)) {
                    Image(systemName: "person.2")
                }
                .accessibilityLabel("People")
                Button { showSettings = true } label: { Image(systemName: "ellipsis.circle") }
                    .accessibilityLabel("Conversation settings")
            }
        }
        .task { await model.run() }
        .onDisappear { Task { await app.refresh() } }
        .sheet(isPresented: $showSettings) {
            ConversationSettings(model: model, epoch: model.messages.last?.epoch) { done in
                showSettings = false
                if done == "relay" { showRelay = true }
                if done == "rotated" { notice = "New epoch started" }
            }
            .presentationDetents([.medium, .large])
        }
        .sheet(isPresented: $showRelay) {
            RelaySheet(core: model.core, circle: model.circle, current: summary?.relay ?? "") {
                showRelay = false
                Task {
                    await app.refresh()
                    await model.poll()
                }
            }
        }
    }
}

struct Bubble: View {
    let message: ChatMessage

    var body: some View {
        VStack(alignment: message.mine ? .trailing : .leading, spacing: 2) {
            Text(meta).font(.caption2).foregroundStyle(.secondary)
            Text(message.text)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(message.mine ? Color.accentColor.opacity(0.18) : Color.secondary.opacity(0.12),
                            in: RoundedRectangle(cornerRadius: 14))
        }
        .frame(maxWidth: .infinity, alignment: message.mine ? .trailing : .leading)
    }

    private var meta: String {
        let who = message.mine ? "you" : "seat \(message.sender + 1)"
        let pending = message.pending ? " · not sent yet" : ""
        return "\(who) · epoch \(message.epoch)\(pending)"
    }
}

struct ConversationSettings: View {
    @ObservedObject var model: ChatModel
    let epoch: UInt32?
    let done: (String) -> Void
    @State private var confirmRotate = false

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Text("End-to-end encrypted").font(.headline)
                    Text("The circle root from your ceremony seeds the keys and ChaCha20-Poly1305 encrypts. The relay sees ciphertext and traffic shape only.")
                        .foregroundStyle(.secondary)
                    if let epoch { Text("Current keys: epoch \(epoch)").font(.footnote) }
                    Text("A sender label identifies a message chain, not proof of which member typed it.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
                Section {
                    Button("Start a new epoch") { confirmRotate = true }
                } footer: {
                    Text("Starting a new epoch moves everyone to new keys from here on. It does not protect older messages.")
                }
                Section {
                    Button("Relay address") { done("relay") }
                }
            }
            .navigationTitle("Conversation settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { done("") } }
            }
            .confirmationDialog("Start a new epoch?", isPresented: $confirmRotate, titleVisibility: .visible) {
                Button("Start a new epoch") {
                    Task { if await model.rotate() { done("rotated") } }
                }
            }
        }
    }
}

/// The relay this circle's messages travel through. Every member must use the same one.
struct RelaySheet: View {
    let core: Core
    let circle: String
    let current: String
    let done: () -> Void
    @State private var address = ""
    @State private var fallback = ""
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("Messages travel through a relay that every member can reach. It only ever sees encrypted messages. The app ships with relay.cahoots.network. Every member of this circle must use the same relay, so change it together.")
                        .foregroundStyle(.secondary)
                }
                Section("Relay address") {
                    TextField("https://relay.example.com", text: $address)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section {
                    Button("Save and connect") { Task { await save(address) } }
                    if current != fallback && !fallback.isEmpty {
                        Button("Use the default relay") { Task { await save(fallback) } }
                    }
                    StatusLine(text: error)
                }
            }
            .navigationTitle("Relay")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { done() } }
            }
            .task {
                address = current
                fallback = (try? await core.run { $0.defaultRelay() }) ?? ""
            }
        }
    }

    private func save(_ url: String) async {
        do {
            try await core.run { [circle] in try $0.setRelay(circle: circle, url: url) }
            done()
        } catch {
            self.error = plain(error)
        }
    }
}
