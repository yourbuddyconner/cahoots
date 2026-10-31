import SwiftUI

/// A channel link opened from outside: what the channel is, read and checked from its relay,
/// and Open to read it here.
struct JoinChannelView: View {
    let core: Core
    /// The whole link, `https://cahoots.network/app/#/ch/<host>/<id>[/<secret>]`.
    let link: String
    @EnvironmentObject private var app: AppModel
    @State private var preview: ChannelPreview?
    @State private var reading = true
    @State private var busy = false
    @State private var error: String?

    /// The host the link names, for the moment before the preview arrives.
    private var host: String {
        guard let f = URL(string: link)?.fragment else { return "the relay" }
        let parts = f.split(separator: "/").map(String.init)
        return parts.count >= 3 ? parts[1] : "the relay"
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                VStack(alignment: .leading, spacing: Space.s) {
                    Text("A channel on \(preview?.relay ?? host)")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Color.muted)
                    Text(preview?.name ?? (reading ? "Opening the channel…" : "Couldn’t open this channel"))
                        .serifTitle(.largeTitle)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                }
                if let preview {
                    VStack(alignment: .leading, spacing: Space.m) {
                        Text(preview.hasSecret ? ChannelCopy.sentence(preview.policyLine) : ChannelCopy.closed)
                            .font(.body).foregroundStyle(Color.ink)
                        HStack(alignment: .firstTextBaseline, spacing: Space.s) {
                            Image(systemName: "lock").accessibilityHidden(true)
                            Text(ChannelCopy.label + ".")
                        }
                        .font(.subheadline).foregroundStyle(Color.muted)
                        if preview.hasSecret, app.channel(preview.channelId) != nil {
                            Text("This channel is already on your phone. Tap Open to use this link and update your access if the link has changed.")
                                .font(.subheadline).foregroundStyle(Color.muted)
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                } else if reading {
                    ProgressView().frame(maxWidth: .infinity, alignment: .leading)
                }
                StatusLine(text: error)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, Space.l)
            .padding(.top, Space.l)
        }
        .paperBackground()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .actionBar(shown: preview?.hasSecret == true) {
            Button { Task { await open() } } label: { Working(busy: busy, title: "Open") }
                .buttonStyle(.filled)
                .disabled(busy)
                .accessibilityIdentifier("Open")
        }
        .task(id: link) { await read() }
    }

    private func read() async {
        reading = true
        defer { reading = false }
        do {
            preview = try await core.run { [link] in try $0.channelPreview(url: link) }
            error = nil
        } catch {
            self.error = plain(error)
        }
    }

    private func open() async {
        guard !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let summary = try await core.run { [link] in try $0.channelJoin(url: link) }
            await app.refresh()
            // The channel takes the join screen's place.
            if case .joinChannel = app.path.last { app.path.removeLast() }
            app.path.append(.channel(summary.id))
        } catch {
            self.error = plain(error)
        }
    }
}
