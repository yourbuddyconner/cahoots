import SwiftUI

/// The owner's list of people asking to post, each with Allow and Refuse.
struct ChannelRequestsView: View {
    let core: Core
    let id: String
    @EnvironmentObject private var app: AppModel
    @State private var requests: [ChannelRequest]?
    @State private var working: String?
    @State private var status: String?
    @State private var notice: String?
    @AppStorage private var autoAllow: Bool

    init(core: Core, id: String) {
        self.core = core
        self.id = id
        _autoAllow = AppStorage(wrappedValue: false, Self.autoAllowKey(id))
    }

    /// The UserDefaults key of a channel's "Allow automatically" toggle.
    static func autoAllowKey(_ id: String) -> String { "channel.\(id).autoAllow" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                ScreenHeader(title: "Who wants to post.",
                             detail: "Each person typed their own name. Allow gives them a grant to post. You can revoke it from Members.",
                             style: .title)
                PaperSection {
                    if let requests {
                        if requests.isEmpty {
                            Text("No requests are waiting.")
                                .font(.subheadline).foregroundStyle(Color.muted)
                                .frame(maxWidth: .infinity, minHeight: 52, alignment: .leading)
                        }
                        ForEach(Array(requests.enumerated()), id: \.element.id) { i, r in
                            if i > 0 { Hairline() }
                            row(r)
                        }
                    } else {
                        ProgressView().frame(maxWidth: .infinity, minHeight: 52)
                    }
                }
                PaperSection(footer: "Requests that came with the live link are let in while this channel is open here.") {
                    Toggle(isOn: $autoAllow) {
                        Text("Allow automatically what the policy already permits")
                            .foregroundStyle(Color.ink)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .tint(Color.brand)
                    .padding(.vertical, Space.m)
                }
                StatusLine(text: notice, problem: false)
                StatusLine(text: status)
            }
            .padding(Space.l)
        }
        .paperBackground()
        .navigationTitle("Requests")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task(id: autoAllow) {
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: .seconds(10))
            }
        }
    }

    /// With the toggle on: let in each request whose every asked-for action the live link
    /// grants (`channelAutoAllow`). Any other request (asking to post in members post, an old
    /// link) stays in the list for the owner. How many were let in.
    static func allowPermitted(core: Core, id: String) async -> Int {
        guard UserDefaults.standard.bool(forKey: autoAllowKey(id)) else { return 0 }
        let n = try? await core.run { try $0.channelAutoAllow(id: id) }
        return Int(n ?? 0)
    }

    private func row(_ r: ChannelRequest) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            VStack(alignment: .leading, spacing: 2) {
                Text(r.name).font(.headline).foregroundStyle(Color.ink)
                Text("Asks to \(Self.words(r.actions)).").font(.subheadline).foregroundStyle(Color.muted)
            }
            .accessibilityElement(children: .combine)
            // The owner picks what to give: read, or read and post. Never admit or govern.
            HStack(spacing: Space.s) {
                if Self.asksToPost(r.actions) {
                    Button { Task { await allow(r, "read,post") } } label: {
                        Working(busy: working == r.id + "read,post", title: "Allow to post")
                    }
                    .buttonStyle(.filled)
                    Button { Task { await allow(r, "read") } } label: {
                        Working(busy: working == r.id + "read", title: "Allow to read")
                    }
                    .buttonStyle(.outlined)
                } else {
                    Button { Task { await allow(r, "read") } } label: {
                        Working(busy: working == r.id + "read", title: "Allow to read")
                    }
                    .buttonStyle(.filled)
                }
                Button("Refuse") { Task { await refuse(r) } }
                    .buttonStyle(.outlined)
            }
            .disabled(working != nil)
        }
        .padding(.vertical, Space.m)
    }

    /// Whether a request's actions include posting.
    static func asksToPost(_ actions: String) -> Bool {
        actions.split(separator: ",").contains { $0.trimmingCharacters(in: .whitespaces) == "post" }
    }

    /// "read,post" as "read and post".
    static func words(_ actions: String) -> String {
        let list = actions.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
        return list.count > 1 ? list.dropLast().joined(separator: ", ") + " and " + list.last! : (list.first ?? "read")
    }

    private func load() async {
        do {
            let list = try await core.run { [id] in try $0.channelRequests(id: id) }
            status = nil
            if autoAllow, !list.isEmpty {
                let n = await Self.allowPermitted(core: core, id: id)
                if n > 0 {
                    notice = n == 1 ? "Let in a request the policy permits." : "Let in \(n) requests the policy permits."
                    requests = (try? await core.run { [id] in try $0.channelRequests(id: id) }) ?? list
                    return
                }
            }
            requests = list
        } catch {
            status = plain(error)
            if requests == nil { requests = [] }
        }
    }

    /// Let `r` in for `actions`, "read" or "read,post", as the owner chose.
    private func allow(_ r: ChannelRequest, _ actions: String) async {
        working = r.id + actions
        defer { working = nil }
        do {
            try await core.run { [id] in try $0.channelAllow(id: id, requestId: r.id, actions: actions) }
            notice = "\(r.name) can \(Self.words(actions))."
            await load()
        } catch {
            status = plain(error)
        }
    }

    private func refuse(_ r: ChannelRequest) async {
        working = r.id
        defer { working = nil }
        do {
            try await core.run { [id] in try $0.channelRefuse(id: id, requestId: r.id) }
            await load()
        } catch {
            status = plain(error)
        }
    }
}
