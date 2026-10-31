import SwiftUI

/// A channel link opened from outside: what the channel is, and Open to read it here.
struct JoinChannelView: View {
    let core: Core
    /// The whole link, `https://cahoots.network/app/#/ch/<host>/<id>/<secret>`.
    let link: String
    @EnvironmentObject private var app: AppModel
    @State private var busy = false
    @State private var error: String?

    private var parts: [String] {
        guard let url = URL(string: link), let f = url.fragment else { return [] }
        return f.split(separator: "/").map(String.init)
    }
    private var host: String { parts.count == 4 ? parts[1] : "" }
    private var channelId: String { parts.count == 4 ? parts[2] : "" }
    /// A channel this phone already reads: the link is newer, or the same.
    private var known: ChannelSummary? { app.channel(channelId) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                VStack(alignment: .leading, spacing: Space.s) {
                    Text("Channel").font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
                    Text(known?.name ?? "A channel on \(host)")
                        .serifTitle(.largeTitle)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                }
                VStack(alignment: .leading, spacing: Space.s) {
                    if let circle = known?.circle {
                        Text("Owned by circle \(ChannelCopy.short(circle)), on \(host).")
                            .font(.subheadline).foregroundStyle(Color.muted)
                    } else {
                        Text("On \(host). Channel \(ChannelCopy.short(channelId)).")
                            .font(.subheadline).foregroundStyle(Color.muted)
                    }
                    if let line = known?.policyLine, !line.isEmpty {
                        Text(ChannelCopy.sentence(line)).font(.body).foregroundStyle(Color.ink)
                    }
                    Text("\(ChannelCopy.label).")
                        .font(.subheadline).foregroundStyle(Color.muted)
                }
                .fixedSize(horizontal: false, vertical: true)
                Text(known == nil
                     ? "Open reads the channel from its relay and checks it before showing it. Anyone with this link can read what is posted."
                     : "This phone already reads this channel. Open takes this link, so a newer one keeps it working.")
                    .font(.subheadline).foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
                StatusLine(text: error)
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.l)
        }
        .paperBackground()
        .navigationTitle("Channel")
        .navigationBarTitleDisplayMode(.inline)
        .actionBar {
            Button { Task { await open() } } label: { Working(busy: busy, title: "Open") }
                .buttonStyle(.filled)
                .disabled(busy || channelId.isEmpty)
                .accessibilityIdentifier("Open")
        }
    }

    private func open() async {
        guard !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let summary = try await core.run { [link] in try $0.channelJoin(url: link) }
            await app.refresh()
            app.path = [.channel(summary.id)]
        } catch {
            self.error = plain(error)
        }
    }
}
