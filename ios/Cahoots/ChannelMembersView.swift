import SwiftUI

/// The people the owner let in, each with Revoke.
struct ChannelMembersView: View {
    let core: Core
    let id: String
    @EnvironmentObject private var app: AppModel
    @State private var members: [ChannelMemberInfo]?
    @State private var revoking: ChannelMemberInfo?
    @State private var working: String?
    @State private var status: String?
    @State private var notice: String?
    @State private var sharing: SharedLink?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                ScreenHeader(title: "People you let in.",
                             detail: "Each holds a grant from your seat. Revoking one ends it and starts new keys.",
                             style: .title)
                PaperSection {
                    if let members {
                        if members.isEmpty {
                            Text("No one else is a member yet.")
                                .font(.subheadline).foregroundStyle(Color.muted)
                                .frame(maxWidth: .infinity, minHeight: 52, alignment: .leading)
                        }
                        ForEach(Array(members.enumerated()), id: \.element.grantId) { i, m in
                            if i > 0 { Hairline() }
                            row(m)
                        }
                    } else {
                        ProgressView().frame(maxWidth: .infinity, minHeight: 52)
                    }
                }
                StatusLine(text: notice, problem: false)
                StatusLine(text: status)
            }
            .padding(Space.l)
        }
        .paperBackground()
        .navigationTitle("Members")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .confirmationDialog("Revoke \(revoking?.name ?? "this member")? Anyone who holds the link gets a new one, and new keys start. \(ChannelCopy.newLink)",
                            isPresented: Binding(get: { revoking != nil }, set: { if !$0 { revoking = nil } }),
                            titleVisibility: .visible) {
            Button("Revoke", role: .destructive) {
                guard let m = revoking else { return }
                Task { await revoke(m) }
            }
        }
        .sheet(item: $sharing) { s in
            ChannelShareSheet(name: app.channel(id)?.name ?? "Channel", shared: s,
                              policyLine: app.channel(id)?.policyLine ?? "") { sharing = nil }
        }
    }

    private func row(_ m: ChannelMemberInfo) -> some View {
        HStack(alignment: .center, spacing: Space.m) {
            VStack(alignment: .leading, spacing: 2) {
                Text(m.name).font(.headline).foregroundStyle(Color.ink)
                Text("Can \(ChannelRequestsView.words(m.actions)). Grant \(ChannelCopy.short(m.grantId)).")
                    .font(.subheadline).foregroundStyle(Color.muted)
            }
            .accessibilityElement(children: .combine)
            Spacer(minLength: Space.s)
            Button { revoking = m } label: {
                if working == m.grantId { ProgressView() } else { Text("Revoke") }
            }
            .buttonStyle(TextButtonStyle(full: false))
            .disabled(working != nil)
        }
        .padding(.vertical, Space.m)
    }

    private func load() async {
        do {
            members = try await core.run { [id] in try $0.channelMembers(id: id) }
        } catch {
            status = plain(error)
            if members == nil { members = [] }
        }
    }

    private func revoke(_ m: ChannelMemberInfo) async {
        working = m.grantId
        defer { working = nil }
        do {
            let link = try await core.run { [id] in try $0.channelRevoke(id: id, grantId: m.grantId) }
            status = nil
            await load()
            await app.refresh()
            if link.isEmpty {
                status = nil
                notice = "\(m.name) is removed and new keys started."
            } else {
                sharing = SharedLink(link: link, note: "\(m.name) is removed and new keys started. \(ChannelCopy.newLink)")
            }
        } catch {
            status = plain(error)
        }
    }
}
