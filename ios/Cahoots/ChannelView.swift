import SwiftUI

/// A channel's screen: its posts, a composer for those who may post, and the relay read every
/// three seconds while open.
@MainActor
final class ChannelModel: ObservableObject {
    let core: Core
    let id: String
    @Published private(set) var summary: ChannelSummary?
    @Published private(set) var messages: [ChannelMessage] = []
    @Published var draft = ""
    @Published private(set) var sending = false
    @Published var status: String?
    private var rounds = 0

    init(core: Core, id: String) {
        self.core = core
        self.id = id
    }

    var isOwner: Bool { summary?.role == .owner }

    func load() async {
        if let s = try? await core.run({ [id] in try $0.channel(id: id) }) { summary = s }
        messages = (try? await core.run { [id] in try $0.channelHistory(id: id) }) ?? messages
    }

    func poll() async {
        do {
            let fresh = try await core.run { [id] in try $0.channelPoll(id: id) }
            core.store.relax()
            if status != nil, !sending { status = nil }
            if !fresh.isEmpty { messages = (try? await core.run { [id] in try $0.channelHistory(id: id) }) ?? messages }
            if let s = try? await core.run({ [id] in try $0.channel(id: id) }) { summary = s }
            // The owner's "Allow automatically" toggle works while the channel is open here.
            if isOwner, summary?.dead != true {
                rounds += 1
                if rounds % 5 == 1, await ChannelRequestsView.allowPermitted(core: core, id: id) > 0 {
                    status = "Let in a request the policy permits."
                }
            }
        } catch PhoneError.Network(let m) where m == UrlSessionHttp.unreachable {
            status = "\(m) Trying again."
        } catch {
            // A closed channel's own words are in the bottom bar already.
            status = summary?.closed == true && summary?.canPost != true ? nil : plain(error)
            if let s = try? await core.run({ [id] in try $0.channel(id: id) }) { summary = s }
        }
    }

    func run() async {
        await load()
        while !Task.isCancelled {
            if summary?.dead != true { await poll() }
            try? await Task.sleep(for: .seconds(3))
        }
    }

    func send() async {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !sending else { return }
        sending = true
        defer { sending = false }
        do {
            try await core.run { [id] in try $0.channelSend(id: id, text: text) }
            draft = ""
            status = nil
        } catch {
            status = "Not sent: \(plain(error))"
        }
        await load()
    }

    func askToPost(name: String) async -> Bool {
        do {
            try await core.run { [id] in try $0.channelAskToPost(id: id, name: name) }
            await load()
            return true
        } catch {
            status = plain(error)
            return false
        }
    }

    /// Run an owner's action, its error in the status line.
    func attempt<T>(_ f: @escaping (Phone, String) throws -> T) async -> T? {
        do {
            let r = try await core.run { [id] in try f($0, id) }
            await load()
            return r
        } catch {
            status = plain(error)
            return nil
        }
    }
}

/// A link to show in the share sheet, with a note above it when it replaces an old one.
struct SharedLink: Identifiable {
    let link: String
    var note: String?
    var id: String { link }
}

struct ChannelView: View {
    @EnvironmentObject private var app: AppModel
    @StateObject private var model: ChannelModel
    @State private var sharing: SharedLink?
    @State private var notice: String?
    @State private var confirmRetire = false
    @State private var confirmRemove = false
    @State private var asking = false
    @FocusState private var composing: Bool

    init(core: Core, id: String) {
        _model = StateObject(wrappedValue: ChannelModel(core: core, id: id))
    }

    private var summary: ChannelSummary? { model.summary ?? app.channel(model.id) }
    private var name: String { summary?.name ?? "Channel" }
    private var canSend: Bool {
        !model.sending && !model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        ScrollViewReader { reader in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    labelLine
                    if model.messages.isEmpty {
                        VStack(alignment: .leading, spacing: Space.m) {
                            Text("Nothing posted yet.").serifTitle(.title)
                            Text(model.isOwner ? "Write the first post. Everyone with the link reads it."
                                               : "Posts show up here as they arrive.")
                                .foregroundStyle(Color.muted)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .padding(.top, Space.xxl)
                    }
                    ForEach(Array(model.messages.enumerated()), id: \.element.id) { i, m in
                        let prev = i > 0 ? model.messages[i - 1] : nil
                        Group {
                            if m.note || m.refused {
                                ChannelNote(message: m)
                            } else {
                                ChannelBubble(message: m, first: prev.map {
                                    $0.note || $0.refused || $0.sender != m.sender || $0.mine != m.mine
                                } ?? true)
                            }
                        }
                        .id(m.id)
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
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 3) {
                    Text(name).font(.system(.headline, design: .serif)).foregroundStyle(Color.ink)
                    Text(model.isOwner ? "Your channel" : "On \(summary?.relay ?? "the relay")")
                        .font(.caption).foregroundStyle(Color.muted)
                }
                .accessibilityElement(children: .combine)
            }
            ToolbarItem(placement: .topBarTrailing) { menu }
        }
        .actionBar { bar }
        .task(id: model.id) { await model.run() }
        .onAppear {
            if app.shareOnOpen == model.id {
                app.shareOnOpen = nil
                Task { await share(note: nil) }
            }
        }
        .onDisappear { Task { await app.refresh() } }
        .sheet(item: $sharing) { s in
            ChannelShareSheet(name: name, shared: s, policyLine: summary?.policyLine ?? "") { sharing = nil }
        }
        .sheet(isPresented: $asking) {
            AskToPostSheet(model: model) { asking = false }
                .presentationDetents([.medium])
        }
        .confirmationDialog("Retire this link? Everyone reading by it stops at the next keys. People you let in keep reading.",
                            isPresented: $confirmRetire, titleVisibility: .visible) {
            Button("Retire the link", role: .destructive) {
                Task {
                    if let link = await model.attempt({ try $0.channelRetireLink(id: $1) }) {
                        sharing = SharedLink(link: link, note: "The link is retired. \(ChannelCopy.newLink)")
                    }
                }
            }
        }
        .confirmationDialog(model.isOwner
                                ? "Remove \(name) from this phone? Its keys go with it, and no one can post as its owner again."
                                : "Remove \(name) from this phone? Its keys go with it.",
                            isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove from this phone", role: .destructive) {
                Task {
                    do { try await app.deleteChannel(model.id) } catch { model.status = plain(error) }
                }
            }
        }
    }

    /// The label line: what the relay keeps, and who reads and posts.
    private var labelLine: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Label(ChannelCopy.label, systemImage: "lock")
                .font(.footnote.weight(.medium))
                .foregroundStyle(Color.ink)
            if let line = summary?.policyLine, !line.isEmpty {
                Text(ChannelCopy.sentence(line))
                    .font(.footnote)
                    .foregroundStyle(Color.muted)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.bottom, Space.s)
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var bar: some View {
        if let notice { StatusLine(text: notice, problem: false) }
        StatusLine(text: model.status)
        if summary?.dead == true {
            StateNote(text: ChannelCopy.dead, problem: true)
        } else if summary?.canPost == true {
            composer
        } else if summary?.waiting == true {
            StateNote(text: ChannelCopy.waiting, problem: false)
        } else if summary?.closed == true {
            StateNote(text: ChannelCopy.closed, problem: true)
        } else if let summary {
            StateNote(text: summary.mayAskToPost ? "You can read this channel. Ask the owner to let you post."
                                                 : ChannelCopy.readOnly,
                      problem: false)
            if summary.mayAskToPost {
                Button("Ask to post") { asking = true }
                    .buttonStyle(.outlined)
            }
        }
    }

    private var menu: some View {
        Menu {
            if model.isOwner {
                Button { Task { await share(note: nil) } } label: { Label("Share the link", systemImage: "square.and.arrow.up") }
                if summary?.closed != true {
                    Button { confirmRetire = true } label: { Label("Retire the link", systemImage: "link") }
                }
                Button {
                    Task {
                        if await model.attempt({ try $0.channelNewKeys(id: $1) }) != nil { notice = "New keys started." }
                    }
                } label: { Label("Start new keys", systemImage: "key") }
                Button { app.path.append(.channelMembers(model.id)) } label: { Label("Members", systemImage: "person.2") }
                Button { app.path.append(.channelRequests(model.id)) } label: { Label("Requests", systemImage: "tray") }
            }
            Button(role: .destructive) { confirmRemove = true } label: {
                Label("Remove from this phone", systemImage: "trash")
            }
        } label: {
            Image(systemName: "ellipsis")
        }
        .accessibilityLabel("Channel settings")
    }

    private func share(note: String?) async {
        // A closed channel has no link to share: say so rather than show the phone crate's error.
        if model.isOwner, summary?.closed == true {
            sharing = SharedLink(link: "", note: nil)
            return
        }
        if let link = await model.attempt({ try $0.channelLink(id: $1) }) {
            sharing = SharedLink(link: link, note: note)
        }
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
        .background(Color.paper, in: RoundedRectangle(cornerRadius: 22))
        .overlay(RoundedRectangle(cornerRadius: 22).strokeBorder(Color.line))
    }
}

/// A channel's state in the bottom bar: a sentence, with a mark when it is a problem.
private struct StateNote: View {
    let text: String
    let problem: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.s) {
            Image(systemName: problem ? "exclamationmark.circle" : "hourglass")
                .foregroundStyle(problem ? Color.ink : Color.muted)
                .accessibilityHidden(true)
            Text(text)
                .font(.subheadline)
                .foregroundStyle(problem ? Color.ink : Color.muted)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, Space.s)
        .accessibilityElement(children: .combine)
    }
}

/// One post: who sent it and when above it, mine on the right in blue.
struct ChannelBubble: View {
    let message: ChannelMessage
    let first: Bool

    private var who: String { message.mine ? "You" : message.sender }
    private var time: String {
        Date(timeIntervalSince1970: Double(message.time) / 1000).formatted(date: .omitted, time: .shortened)
    }

    var body: some View {
        VStack(alignment: message.mine ? .trailing : .leading, spacing: Space.xs) {
            HStack(spacing: Space.xs) {
                if first { Text(who).fontWeight(.medium) }
                Text(time)
                if message.announcement {
                    Label(message.approved ? "Announcement" : "Announcement, not approved", systemImage: "megaphone")
                        .labelStyle(.titleAndIcon)
                }
            }
            .font(.caption)
            .foregroundStyle(Color.muted)
            Text(message.text)
                .font(.body)
                .foregroundStyle(message.mine ? Color.paper : Color.ink)
                .padding(.horizontal, Space.m + 2)
                .padding(.vertical, 9)
                .background(bubble)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: message.mine ? .trailing : .leading)
                .padding(message.mine ? .leading : .trailing, 48)
        }
        .padding(.top, first ? Space.m : Space.xs)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(who) at \(time)\(message.announcement ? (message.approved ? ", announcement" : ", announcement, not approved") : ""). \(message.text)")
    }

    @ViewBuilder
    private var bubble: some View {
        let shape = RoundedRectangle(cornerRadius: 18, style: .continuous)
        if message.mine {
            shape.fill(Color.brand)
        } else {
            shape.fill(Color.paper).overlay(shape.strokeBorder(Color.ink.opacity(message.announcement ? 0.4 : 0.14)))
        }
    }
}

/// A line from the app between posts: a member removed, or a message that did not check out.
private struct ChannelNote: View {
    let message: ChannelMessage

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.s) {
            Image(systemName: message.refused ? "exclamationmark.circle" : "info.circle")
                .accessibilityHidden(true)
            Text(message.refused ? "Not shown. \(message.text)" : message.text)
                .fixedSize(horizontal: false, vertical: true)
        }
        .font(.footnote)
        .foregroundStyle(Color.muted)
        .frame(maxWidth: .infinity, alignment: .center)
        .multilineTextAlignment(.center)
        .padding(.vertical, Space.m)
        .accessibilityElement(children: .combine)
    }
}

/// The link and what forwarding it means, with the system share sheet and a copy button.
struct ChannelShareSheet: View {
    let name: String
    let shared: SharedLink
    let policyLine: String
    let done: () -> Void
    @State private var copied = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xl) {
                    if shared.link.isEmpty {
                        ScreenHeader(title: "\(name) is ready.", detail: ChannelCopy.noLink, style: .title)
                    } else {
                        ScreenHeader(title: shared.note == nil ? "Share the link." : "A new link.",
                                     detail: ChannelCopy.linkLimit, style: .title)
                    }
                    if let note = shared.note { StatusLine(text: note, problem: false) }
                    if !shared.link.isEmpty {
                        Text(shared.link)
                            .font(.footnote.monospaced())
                            .foregroundStyle(Color.ink)
                            .textSelection(.enabled)
                            .padding(Space.m)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.paper, in: RoundedRectangle(cornerRadius: Radius.control))
                            .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.line))
                            .accessibilityIdentifier("channel link")
                    }
                    if !policyLine.isEmpty {
                        Text(ChannelCopy.sentence(policyLine)).font(.subheadline).foregroundStyle(Color.muted)
                    }
                }
                .padding(Space.l)
            }
            .paperBackground()
            .navigationTitle(name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { done() } }
            }
            .actionBar(shown: !shared.link.isEmpty) {
                if let url = URL(string: shared.link) {
                    ShareLink(item: url, subject: Text(name), message: Text(ChannelCopy.linkLimit)) {
                        Text("Share the link")
                    }
                    .buttonStyle(.filled)
                }
                Button(copied ? "Copied" : "Copy the link") {
                    UIPasteboard.general.string = shared.link
                    copied = true
                }
                .buttonStyle(.outlined)
            }
        }
    }
}

/// A reader asks the owner to let them post, under a name they choose.
private struct AskToPostSheet: View {
    @ObservedObject var model: ChannelModel
    let done: () -> Void
    @State private var name = ""
    @State private var busy = false

    private var trimmed: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: Space.xl) {
                ScreenHeader(title: "Ask to post.",
                             detail: "Your name is shown to the owner when you ask, and to members beside your posts.",
                             style: .title2)
                TextField("How the owner will know you", text: $name)
                    .submitLabel(.send)
                    .paperField()
                    .onSubmit { Task { await ask() } }
                StatusLine(text: model.status)
                Spacer()
                Button { Task { await ask() } } label: { Working(busy: busy, title: "Ask to post") }
                    .buttonStyle(.filled)
                    .disabled(trimmed.isEmpty || busy)
            }
            .padding(Space.l)
            .paperBackground()
            .navigationTitle("Ask to post")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { done() } }
            }
        }
    }

    private func ask() async {
        guard !trimmed.isEmpty, !busy else { return }
        busy = true
        defer { busy = false }
        if await model.askToPost(name: trimmed) { done() }
    }
}
