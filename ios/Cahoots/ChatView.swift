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
    /// Each seat's face, drawn.
    @Published private(set) var faces: [UInt8: Data] = [:]
    /// The circle's members as last read, with the names and faces on their cards.
    @Published private(set) var members: [Member] = []
    /// This circle shows a face of its own for this phone, not the phone's face.
    @Published private(set) var ownFace = false
    /// What became of the last change to this circle's face, when it needs saying.
    @Published var faceNote: String?
    /// A circle from a badge this phone is paired with: its seat is the badge's.
    @Published private(set) var card = false
    /// A card circle whose chat waits for keys only a tap on the card can bring.
    @Published private(set) var needsTap = false
    /// A tap for the keys is under way.
    @Published private(set) var tapping = false

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
        } catch PhoneError.Network(let m) where m == UrlSessionHttp.unreachable {
            connected = false
            status = "\(m) Trying again."
        } catch {
            connected = false
            status = plain(error)
        }
    }

    /// Read the members and their faces again: a card read by the last poll may change them.
    func loadMembers() async {
        guard let read = try? await core.run({ [circle] phone in
            let members = try phone.circleMembers(circle: circle)
            let faces = Dictionary(uniqueKeysWithValues: members.map {
                ($0.seat, phone.facePixels(seed: $0.faceSeed))
            })
            return (members, faces)
        }) else { return }
        if read.0 != members { members = read.0 }
        if read.1 != faces { faces = read.1 }
    }

    func loadOwnFace() async {
        ownFace = (try? await core.run { [circle] in try $0.circleHasOwnFace(circle: circle) }) ?? false
    }

    /// Give this circle a face of its own, freshly drawn, or go back to the phone's face.
    func setOwnFace(_ fresh: Bool) async {
        faceNote = nil
        do {
            try await core.run { [circle] in try $0.setCircleFace(circle: circle, fresh: fresh) }
        } catch PhoneError.Locked {
            faceNote = "Saved on this phone, but not shared yet. Open Settings and tap Share again."
        } catch {
            faceNote = plain(error)
        }
        await loadOwnFace()
        await loadMembers()
    }

    /// Read again whether the chat waits for a tap. Only a card circle ever does.
    func checkTap() async {
        guard let summary = try? await core.run({ [circle] phone in
            phone.circles().first { $0.id == circle }
        }) else { return }
        card = AppModel.isCard(summary)
        needsTap = summary.needsTap
    }

    /// Tap the card for the keys the chat waits for, then read the relay with them.
    func tapForKeys() async {
        guard !tapping else { return }
        tapping = true
        defer { tapping = false }
        do {
            try await core.run { [circle] in try $0.tapForKeys(circle: circle) }
            status = nil
            await poll()
            await load()
        } catch {
            status = plain(error)
        }
        await checkTap()
    }

    func run() async {
        await load()
        await loadOwnFace()
        await checkTap()
        while !Task.isCancelled {
            await poll()
            await loadMembers()
            if card { await checkTap() }
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
        if card { await checkTap() }
        await load()
    }

    /// Move everyone to new keys. Returns what to tell the person, or nil when it failed.
    func reset() async -> String? {
        do {
            try await core.run { [circle] in try $0.bumpChatGeneration(circle: circle) }
            let waiting = (try? await core.run { [circle] in $0.chatResetWaiting(circle: circle) }) ?? false
            await load()
            return waiting
                ? "Reset saved. It reaches the others when the relay answers."
                : "Keys were reset for everyone."
        } catch {
            status = plain(error)
            return nil
        }
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
    @FocusState private var composing: Bool

    init(core: Core, circle: String) {
        _model = StateObject(wrappedValue: ChatModel(core: core, circle: circle))
    }

    private var summary: CircleSummary? { app.circle(model.circle) }
    private var canSend: Bool {
        !model.sending && !model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        ScrollViewReader { reader in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if model.messages.isEmpty {
                        ChatEmpty(faces: model.members.compactMap { model.faces[$0.seat] })
                    }
                    ForEach(Array(model.messages.enumerated()), id: \.element.id) { i, m in
                        let prev = i > 0 ? model.messages[i - 1] : nil
                        let next = i + 1 < model.messages.count ? model.messages[i + 1] : nil
                        if m.kind == "bump" {
                            BumpLine(text: m.text).id(m.id)
                        } else {
                        if let prev, prev.epoch != m.epoch, prev.kind != "bump" {
                            EpochDivider()
                        }
                        Bubble(message: m,
                               face: model.faces[m.sender],
                               senderName: app.seatName(circle: model.circle, seat: m.sender),
                               first: prev.map { !Self.sameRun($0, m) } ?? true,
                               last: next.map { !Self.sameRun(m, $0) } ?? true)
                            .id(m.id)
                        }
                    }
                }
                .padding(.horizontal, Space.l)
                .padding(.vertical, Space.m)
            }
            .defaultScrollAnchor(.bottom)
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: model.messages.last?.id) { _, last in
                if let last { withAnimation(.snappy) { reader.scrollTo(last, anchor: .bottom) } }
            }
        }
        .paperBackground()
        .navigationTitle(summary?.name ?? "Chat")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                NavigationLink(value: Route.people(model.circle)) {
                    VStack(spacing: 3) {
                        Text(summary?.name ?? "Chat")
                            .font(.system(.headline, design: .serif))
                            .foregroundStyle(Color.ink)
                        HStack(spacing: Space.xs) {
                            Text(others ?? "\(summary?.members ?? 0) people").lineLimit(1)
                            Image(systemName: "chevron.right").font(.caption2.weight(.semibold))
                        }
                        .font(.caption).foregroundStyle(Color.muted)
                    }
                    .padding(.horizontal, Space.s)
                    .frame(minHeight: 44)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("People in \(summary?.name ?? "this circle")")
                .accessibilityValue(others ?? "\(summary?.members ?? 0) people")
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button { showSettings = true } label: { Image(systemName: "ellipsis") }
                    .accessibilityLabel("Conversation settings")
            }
        }
        .actionBar {
            if let notice { StatusLine(text: notice, problem: false) }
            StatusLine(text: model.status)
            if model.needsTap {
                Button { Task { await model.tapForKeys(); await app.refresh() } } label: {
                    Working(busy: model.tapping, title: "Tap your card to catch up")
                }
                .buttonStyle(.filled)
                .disabled(model.tapping)
                .accessibilityLabel("Tap your card to catch up")
                .accessibilityIdentifier("tap-for-keys")
            } else if summary?.hasRoot == false && !model.card {
                Text("This circle uses an older format that doesn’t support chat. Create a new circle together to start chatting.")
                    .font(.subheadline).foregroundStyle(Color.muted)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                composer
            }
        }
        .task { await model.run() }
        .onChange(of: model.members) { _, members in app.noteMembers(members, circle: model.circle) }
        .onDisappear { Task { await app.refresh() } }
        .sheet(isPresented: $showSettings) {
            ConversationSettings(model: model, epoch: model.messages.last?.epoch) { done in
                showSettings = false
                if done == "people" { app.path.append(.people(model.circle)) }
                if done == "relay" { showRelay = true }
                if done.hasPrefix("reset:") { notice = String(done.dropFirst(6)) }
                if done == "rotated" { notice = "Future messages will use new encryption keys." }
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

    /// The names of the other members, for the header, once any of them has a name.
    private var others: String? {
        app.others(circle: model.circle, seats: model.members.filter { !$0.mine }.map(\.seat))
    }

    static func sameRun(_ a: ChatMessage, _ b: ChatMessage) -> Bool {
        a.kind != "bump" && b.kind != "bump" && a.mine == b.mine && a.sender == b.sender && a.epoch == b.epoch
    }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: Space.s) {
            TextField("Message", text: $model.draft, axis: .vertical)
                .lineLimit(1...5)
                .focused($composing)
                .padding(.leading, Space.l)
                .padding(.vertical, 11)
                .foregroundStyle(Color.ink)
                .accessibilityIdentifier("message")
                .onSubmit { Task { await model.send() } }
            Button {
                Task { await model.send() }
            } label: {
                ZStack {
                    Circle().fill(canSend ? Color.brand : Color.line)
                    if model.sending {
                        ProgressView().tint(Color.paper)
                    } else {
                        Image(systemName: "arrow.up")
                            .font(.body.weight(.semibold))
                            .foregroundStyle(canSend ? Color.paper : Color.muted)
                    }
                }
                .frame(width: 34, height: 34)
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!canSend)
            .accessibilityLabel("Send")
            .accessibilityIdentifier("send")
        }
        .padding(.trailing, 2)
        .modifier(ComposerSurface())
    }
}

/// The composer's field: glass on iOS 26, paper in a hairline before.
private struct ComposerSurface: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.glassEffect(.regular, in: RoundedRectangle(cornerRadius: 22))
        } else {
            content
                .background(Color.paper, in: RoundedRectangle(cornerRadius: 22))
                .overlay(RoundedRectangle(cornerRadius: 22).strokeBorder(Color.line))
        }
    }
}

/// A circle with no messages yet.
private struct ChatEmpty: View {
    let faces: [Data]

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            if !faces.isEmpty { FaceRow(faces: faces, size: 32, limit: 8).padding(.bottom, Space.xs) }
            Text("Start with a hello.").serifTitle(.title)
            Text("Messages are encrypted on your phone before the relay delivers them. Only this circle can read them.")
                .font(.body).foregroundStyle(Color.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.top, Space.xxl)
        .padding(.bottom, Space.xl)
    }
}

/// A note in the transcript that a seat reset the keys, centred in the muted style.
private struct BumpLine: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption.weight(.medium))
            .foregroundStyle(Color.muted)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .padding(.vertical, Space.l)
    }
}

/// Where the circle moved to new keys.
private struct EpochDivider: View {
    var body: some View {
        HStack(spacing: Space.m) {
            Hairline()
            Text("Encryption keys renewed")
                .font(.caption.weight(.medium))
                .foregroundStyle(Color.muted)
                .fixedSize()
            Hairline()
        }
        .padding(.vertical, Space.l)
    }
}

struct Bubble: View {
    let message: ChatMessage
    let face: Data?
    var senderName: String? = nil
    /// The first and last message of one sender's run.
    let first: Bool
    let last: Bool

    private static let faceSize: CGFloat = 24

    var body: some View {
        VStack(alignment: message.mine ? .trailing : .leading, spacing: Space.xs) {
            if first && !message.mine {
                Text(senderName ?? "Seat \(message.sender + 1)")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(Color.muted)
                    .padding(.leading, Self.faceSize + Space.s + Space.m)
                    .accessibilityHidden(true)
            }
            HStack(alignment: .bottom, spacing: Space.s) {
                if !message.mine {
                    if last {
                        FaceView(pixels: face, size: Self.faceSize)
                    } else {
                        Color.clear.frame(width: Self.faceSize, height: 1)
                    }
                }
                Text(message.text)
                    .font(.body)
                    .foregroundStyle(message.mine ? Color.paper : Color.ink)
                    .padding(.horizontal, Space.m + 2)
                    .padding(.vertical, 9)
                    .background(bubble)
                    .opacity(message.pending ? 0.6 : 1)
                    .textSelection(.enabled)
            }
            .frame(maxWidth: .infinity, alignment: message.mine ? .trailing : .leading)
            .padding(message.mine ? .leading : .trailing, 48)
            if message.pending {
                Text("Not sent yet")
                    .font(.caption)
                    .foregroundStyle(Color.muted)
            }
        }
        .padding(.top, first ? Space.m : 2)
        .accessibilityElement(children: .contain)
        .accessibilityHint(message.mine ? "Sent by you" : "From \(senderName ?? "seat \(message.sender + 1)")")
    }

    @ViewBuilder
    private var bubble: some View {
        let shape = RoundedRectangle(cornerRadius: 18, style: .continuous)
        if message.mine {
            shape.fill(Color.brand)
        } else {
            shape.fill(Color.paper).overlay(shape.strokeBorder(Color.ink.opacity(0.14)))
        }
    }
}

struct ConversationSettings: View {
    @ObservedObject var model: ChatModel
    let epoch: UInt32?
    let done: (String) -> Void
    @State private var confirmRotate = false
    @State private var confirmReset = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xxl) {
                    PaperSection(footer: "See who is in this circle and check a member’s hello words.") {
                        Button { done("people") } label: {
                            HStack(spacing: Space.m) {
                                Label("People and names", systemImage: "person.2")
                                Spacer()
                                Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                            }
                            .foregroundStyle(Color.ink)
                            .frame(minHeight: 52)
                        }
                        .buttonStyle(RowButtonStyle())
                    }
                    if !model.card {
                        PaperSection(footer: "Show a face drawn only for this circle. Your other circles keep your face from Settings.") {
                            HStack(spacing: Space.m) {
                                FaceView(pixels: model.members.first(where: \.mine).flatMap { model.faces[$0.seat] },
                                         size: 32)
                                Toggle(isOn: Binding(get: { model.ownFace },
                                                     set: { on in Task { await model.setOwnFace(on) } })) {
                                    Text("New face for this circle")
                                        .foregroundStyle(Color.ink)
                                        .fixedSize(horizontal: false, vertical: true)
                                }
                                .tint(Color.brand)
                                .accessibilityIdentifier("circle-face")
                            }
                            .padding(.vertical, Space.m)
                            StatusLine(text: model.faceNote)
                        }
                    }
                    VStack(alignment: .leading, spacing: Space.s) {
                        Text("End-to-end encrypted").serifTitle(.title2)
                        Text("Messages are encrypted before they leave your phone. The relay delivers them without being able to read them. It can still see when messages are sent.")
                            .foregroundStyle(Color.muted)
                            .fixedSize(horizontal: false, vertical: true)
                        if let epoch {
                            Text("Current keys: epoch \(epoch).")
                                .font(.subheadline).foregroundStyle(Color.ink)
                        }
                    }
                    PaperSection(footer: "Renewing the encryption keys changes the keys used for future messages. It does not change the protection of messages already sent.") {
                        Button("Renew encryption keys") { confirmRotate = true }
                            .buttonStyle(TextButtonStyle(full: true))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    PaperSection(footer: "Every member moves to new keys. A device that cannot reach its card reads nothing new until it taps.") {
                        Button("Reset keys for everyone") { confirmReset = true }
                            .buttonStyle(TextButtonStyle(full: true))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    PaperSection(footer: "Sender names help you follow a conversation. They do not independently verify who typed a message.") {
                        Button {
                            done("relay")
                        } label: {
                            HStack {
                                Text("Relay address").foregroundStyle(Color.ink)
                                Spacer()
                                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
                            }
                            .frame(minHeight: 48)
                        }
                        .buttonStyle(RowButtonStyle())
                    }
                }
                .padding(Space.l)
            }
            .paperBackground()
            .navigationTitle("Conversation")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { done("") } }
            }
            .confirmationDialog("Reset keys for everyone?", isPresented: $confirmReset, titleVisibility: .visible) {
                Button("Reset keys for everyone") {
                    Task { if let note = await model.reset() { done("reset:" + note) } }
                }
            }
            .confirmationDialog("Renew encryption keys?", isPresented: $confirmRotate, titleVisibility: .visible) {
                Button("Renew encryption keys") {
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
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xl) {
                    Text("A relay is the server that delivers your encrypted messages. It cannot read them. Cahoots uses relay.cahoots.network by default. If you change this address, everyone in the circle must switch to the same address.")
                        .foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)
                    VStack(alignment: .leading, spacing: Space.s) {
                        Text("Relay address").font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
                        TextField("https://relay.example.com", text: $address)
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .font(.body.monospaced())
                            .paperField()
                    }
                    StatusLine(text: error)
                    VStack(spacing: Space.s) {
                        Button("Save and connect") { Task { await save(address) } }
                            .buttonStyle(.filled)
                        if current != fallback && !fallback.isEmpty {
                            Button("Use the default relay") { Task { await save(fallback) } }
                                .buttonStyle(.text)
                        }
                    }
                }
                .padding(Space.l)
            }
            .paperBackground()
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
