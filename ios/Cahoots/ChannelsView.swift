import SwiftUI

/// The words the channel screens share, as the web app has them (web/app/channels.js).
enum ChannelCopy {
    static let label = "End-to-end encrypted. The relay cannot read your messages"
    static let linkLimit = "Anyone with this link can read the channel and forward the link to others. Disable it to stop access through that link."
    static let dead = "You no longer have access through this link. Ask the owner for a new link or an invitation."
    static let waiting = "Your request to post is waiting for the owner’s approval."
    static let closed = "This channel is private. Only people approved by the owner can read it."
    static let newLink = "Share the new link with anyone who should keep reading through a link. Approved members keep their access."
    static let readOnly = "You can read this channel. Posting is limited to the owner."
    static let noLink = "This is a private channel with no shared link. Only people you approve can read it."

    /// The policies a channel starts with: the preset number, its name, the one thing that sets
    /// it apart, and the whole policy as a sentence. Announcements with approvals (preset 1) is
    /// left out: it needs approvals from the other seats, which this version cannot collect.
    static let presets: [(number: UInt8, name: String, detail: String, policy: String)] = [
        (0, "Announcements", "Anyone with the link can read. Only you can post.", "Anyone with the link can read. Only you can post."),
        (2, "Group chat", "Anyone with the link can read and request permission to post.", "Anyone with the link can read and request permission to post."),
        (3, "Members post", "Anyone with the link can read. You choose who can post.", "Anyone with the link can read. You choose who can post."),
        (4, "Closed", "You choose who can read and post. There is no shared link.", "You choose who can read and post. There is no shared link."),
    ]

    /// A policy line ("anyone with the link reads, only the owner posts") as a sentence.
    static func sentence(_ line: String) -> String {
        let clarified = line
            .replacingOccurrences(of: "anyone with the link can read and post", with: "anyone with the link can read and request permission to post")
            .replacingOccurrences(of: "members the owner lets in", with: "approved members")
        return clarified.split(separator: ";").map { clause in
            let text = clause.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let first = text.first else { return "" }
            let sentence = first.uppercased() + text.dropFirst()
            return sentence.hasSuffix(".") ? sentence : sentence + "."
        }.joined(separator: " ")
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
                Text("Channels let your circle share messages with a wider audience. You choose who can read and post.")
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
        if channel.waiting { s += " Your request to post is waiting for approval." }
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
                    Text("Choose a name and who can read and post. This channel belongs to \(circle.name). You manage it from this phone.")
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

                    PaperSection(title: "Who can read and post", footer: policy + " Messages are end-to-end encrypted.") {
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
