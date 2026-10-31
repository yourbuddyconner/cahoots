import SwiftUI

/// The words the channel screens share, as the web app has them (web/app/channels.js).
enum ChannelCopy {
    static let label = "End-to-end encrypted; the relay keeps ciphertext only"
    static let linkLimit = "Anyone you forward this link to can read until you retire it."
    static let dead = "This link no longer works. Ask for a new one."
    static let waiting = "Waiting for the owner to let you post."
    static let closed = "Only people the owner lets in can read this channel."
    static let newLink = "Everyone who should keep reading needs this new link."
    static let readOnly = "You can read this channel. Only the owner posts."
    static let noLink = "This channel has no link. Only people you let in can read it."

    /// The policies a channel starts with: the preset number, its name, the one thing that sets
    /// it apart, and the whole policy as a sentence. Announcements with approvals (preset 1) is
    /// left out: it needs approvals from the other seats, which this version cannot collect.
    static let presets: [(number: UInt8, name: String, detail: String, policy: String)] = [
        (0, "Announcements", "Only you post.", "Anyone with the link reads. Only you post."),
        (2, "Group chat", "Anyone with the link can post.", "Anyone with the link reads, and can ask to post."),
        (3, "Members post", "People you let in post.", "Anyone with the link reads. Only people you let in post."),
        (4, "Closed", "Only people you let in can read.", "Only people you let in read and post. There is no link."),
    ]

    /// A policy line ("anyone with the link reads, only the owner posts") as a sentence.
    static func sentence(_ line: String) -> String {
        guard let first = line.first else { return "" }
        let s = first.uppercased() + line.dropFirst()
        return s.hasSuffix(".") ? s : s + "."
    }

    /// The first eight characters of an id or a key, enough to tell two apart.
    static func short(_ id: String) -> String { String(id.prefix(8)) }
}

/// The circles screen's Channels section: every channel on this phone, then a way to start one
/// for each circle.
struct ChannelsSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var starting: CircleSummary?

    var body: some View {
        if !model.channels.isEmpty || model.circles.contains(where: \.hasRoot) {
            VStack(alignment: .leading, spacing: 0) {
                Text("Channels")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Color.muted)
                    .padding(.horizontal, Space.l)
                    .padding(.bottom, Space.s)
                    .accessibilityAddTraits(.isHeader)
                Hairline()
                ForEach(model.channels, id: \.id) { c in
                    NavigationLink(value: Route.channel(c.id)) {
                        ChannelRow(channel: c, owner: c.circle.flatMap { model.circle($0)?.name })
                    }
                    .buttonStyle(RowButtonStyle())
                    Hairline().padding(.leading, Space.l)
                }
                ForEach(model.circles.filter(\.hasRoot), id: \.id) { c in
                    Button { starting = c } label: {
                        HStack(spacing: Space.m) {
                            Image(systemName: "plus").font(.body.weight(.semibold)).accessibilityHidden(true)
                            Text(model.circles.count > 1 ? "Start a channel for \(c.name)" : "Start a channel")
                            Spacer()
                        }
                        .foregroundStyle(Color.brand)
                        .padding(.horizontal, Space.l)
                        .frame(minHeight: 52)
                    }
                    .buttonStyle(RowButtonStyle())
                    Hairline().padding(.leading, Space.l)
                }
                Text("A channel posts to everyone who has its link. Your circle owns it, and this phone holds its keys.")
                    .font(.footnote)
                    .foregroundStyle(Color.muted)
                    .padding(.horizontal, Space.l)
                    .padding(.top, Space.s)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.top, Space.xl)
            .sheet(item: $starting, onDismiss: { model.openStartedChannel() }) { c in
                StartChannelSheet(circle: c) { starting = nil }
            }
        }
    }
}

extension CircleSummary: Identifiable {}

struct ChannelRow: View {
    let channel: ChannelSummary
    /// The owning circle's name, when it is on this phone.
    var owner: String?
    @Environment(\.dynamicTypeSize) private var typeSize

    private var meta: String {
        if channel.dead { return ChannelCopy.dead }
        var s = ChannelCopy.sentence(channel.policyLine)
        if channel.waiting { s += " Waiting for the owner to let you post." }
        if channel.role == .owner, let owner { s += " Owned by \(owner)." }
        return s
    }

    var body: some View {
        HStack(alignment: .center, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.xs) {
                HStack(spacing: Space.s) {
                    if channel.dead {
                        Image(systemName: "exclamationmark.circle")
                            .font(.subheadline)
                            .foregroundStyle(Color.ink)
                            .accessibilityHidden(true)
                    } else if channel.waiting {
                        Image(systemName: "hourglass")
                            .font(.subheadline)
                            .foregroundStyle(Color.muted)
                            .accessibilityHidden(true)
                    }
                    Text(channel.name)
                        .serifTitle(.title3)
                        .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
                }
                Text(meta)
                    .font(.subheadline)
                    .foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: Space.s)
            if channel.unread > 0 {
                Text("\(channel.unread)")
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(Color.paper)
                    .padding(.horizontal, Space.s)
                    .frame(minWidth: 26, minHeight: 26)
                    .background(Color.brand, in: Capsule())
            }
            Image(systemName: "chevron.right")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Color.muted.opacity(0.6))
                .accessibilityHidden(true)
        }
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Channel \(channel.name). \(meta)\(channel.unread > 0 ? " \(channel.unread) unread." : "")")
    }
}

/// Start a channel for a circle: its name and who reads and posts.
struct StartChannelSheet: View {
    let circle: CircleSummary
    let done: () -> Void
    @EnvironmentObject private var app: AppModel
    @State private var name = ""
    @State private var preset: UInt8 = 0
    @State private var busy = false
    @State private var error: String?
    @FocusState private var naming: Bool

    private var trimmed: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// The chosen policy, spelled out under the rows.
    private var policy: String {
        ChannelCopy.presets.first { $0.number == preset }?.policy ?? ""
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xxl) {
                    Text("Give the channel a name and say who may read and post. \(circle.name) owns it, and this phone holds its keys.")
                        .font(.body)
                        .foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)

                    VStack(alignment: .leading, spacing: Space.s) {
                        Text("Name")
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(Color.muted)
                        TextField("garden news, the choir, street parking", text: $name)
                            .textInputAutocapitalization(.words)
                            .autocorrectionDisabled()
                            .submitLabel(.done)
                            .focused($naming)
                            .onSubmit { naming = false }
                            .paperField()
                            .accessibilityLabel("Name")
                            .accessibilityIdentifier("channel name")
                    }

                    PaperSection(title: "Who reads and posts", footer: policy + " The relay keeps encrypted messages only.") {
                        ForEach(Array(ChannelCopy.presets.enumerated()), id: \.element.number) { i, p in
                            if i > 0 { Hairline() }
                            // Choosing a row puts the keyboard away, so the rows are not left under the bar.
                            OptionRow(title: p.name, detail: p.detail, chosen: preset == p.number) {
                                preset = p.number
                                naming = false
                            }
                        }
                    }
                }
                .padding(.horizontal, Space.l)
                .padding(.top, Space.s)
                .padding(.bottom, Space.xl)
            }
            .scrollDismissesKeyboard(.interactively)
            .scrollBounceBehavior(.basedOnSize)
            .paperBackground()
            .navigationTitle("Start a channel")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { done() } }
            }
            .sensoryFeedback(.selection, trigger: preset)
            .actionBar {
                StatusLine(text: error)
                Button { Task { await start() } } label: { Working(busy: busy, title: "Start the channel") }
                    .buttonStyle(.filled)
                    .disabled(trimmed.isEmpty || busy)
                    .accessibilityIdentifier("Start")
            }
        }
    }

    private func start() async {
        guard !trimmed.isEmpty, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            try await app.startChannel(circle: circle.id, name: trimmed, preset: preset)
            done()
        } catch {
            self.error = plain(error)
        }
    }
}
