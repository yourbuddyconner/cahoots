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

    /// The five policies a channel starts with: the preset number, its name, one line on it.
    static let presets: [(UInt8, String, String)] = [
        (0, "Announcements", "Anyone with the link reads. Only you post."),
        (1, "Announcements with approvals", "Anyone with the link reads. A post goes out when enough seats of your circle approve it."),
        (2, "Group chat", "Anyone with the link reads and can ask to post."),
        (3, "Members post", "Anyone with the link reads. People you let in post."),
        (4, "Closed", "Only people you let in read and post."),
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
            .sheet(item: $starting) { c in
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

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xl) {
                    ScreenHeader(title: "Start a channel.",
                                 detail: "Post to everyone who has the link. \(circle.name) owns it, and this phone holds its keys.",
                                 style: .title)
                    VStack(alignment: .leading, spacing: Space.s) {
                        Text("Name").font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
                        TextField("Garden news", text: $name)
                            .focused($naming)
                            .submitLabel(.done)
                            .paperField()
                            .accessibilityIdentifier("channel name")
                    }
                    PaperSection(title: "Who reads and posts",
                                 footer: preset == 4
                                    ? "The relay keeps encrypted messages only. Only people you let in can read."
                                    : "The relay keeps encrypted messages only. Anyone with the link can read.") {
                        ForEach(ChannelCopy.presets, id: \.0) { p in
                            if p.0 > 0 { Hairline() }
                            OptionRow(title: p.1, detail: p.2, chosen: preset == p.0) { preset = p.0 }
                        }
                    }
                    StatusLine(text: error)
                }
                .padding(Space.l)
            }
            .scrollDismissesKeyboard(.interactively)
            .paperBackground()
            .navigationTitle("New channel")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { done() } }
            }
            .actionBar {
                Button { Task { await start() } } label: { Working(busy: busy, title: "Start") }
                    .buttonStyle(.filled)
                    .disabled(trimmed.isEmpty || busy)
                    .accessibilityIdentifier("Start")
            }
            .onAppear { naming = true }
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
