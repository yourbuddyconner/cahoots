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
                ScreenHeader(title: "Approved members",
                             detail: "These people have permission to read or post. Removing a member ends their access to future messages. Messages they already received stay on their device.",
                             style: .title)
                PaperSection {
                    if let members {
                        if members.isEmpty {
                            Text("You haven’t approved any members yet.")
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
        .confirmationDialog("Remove \(revoking?.name ?? "this member")? Their permission will end. If the channel has a shared link, it will be replaced. Share the new link with anyone who still needs it.",
                            isPresented: Binding(get: { revoking != nil }, set: { if !$0 { revoking = nil } }),
                            titleVisibility: .visible) {
            Button("Remove member", role: .destructive) {
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
                Text("Can \(ChannelRequestsView.words(m.actions)).")
                    .font(.subheadline).foregroundStyle(Color.muted)
            }
            .accessibilityElement(children: .combine)
            Spacer(minLength: Space.s)
            Button { revoking = m } label: {
                if working == m.grantId { ProgressView() } else { Text("Remove") }
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
                notice = "\(m.name) has been removed. Future messages will use new encryption keys."
            } else {
                sharing = SharedLink(link: link, note: "\(m.name) has been removed. Future messages will use new encryption keys. \(ChannelCopy.newLink)")
            }
        } catch {
            status = plain(error)
        }
    }
}
