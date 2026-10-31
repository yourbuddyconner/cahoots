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

    /// With the toggle on: allow every request the owner core accepts as it stands, that is one
    /// that came with the live link. Anything it refuses stays in the list. How many were let in.
    static func allowPermitted(core: Core, id: String, requests: [ChannelRequest]? = nil) async -> Int {
        guard UserDefaults.standard.bool(forKey: autoAllowKey(id)) else { return 0 }
        let list: [ChannelRequest]
        if let requests {
            list = requests
        } else {
            guard let read = try? await core.run({ try $0.channelRequests(id: id) }) else { return 0 }
            list = read
        }
        var n = 0
        for r in list where (try? await core.run({ try $0.channelAllow(id: id, requestId: r.id, actions: r.actions) })) != nil {
            n += 1
        }
        return n
    }

    private func row(_ r: ChannelRequest) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            VStack(alignment: .leading, spacing: 2) {
                Text(r.name).font(.headline).foregroundStyle(Color.ink)
                Text("Asks to \(Self.words(r.actions)).").font(.subheadline).foregroundStyle(Color.muted)
            }
            .accessibilityElement(children: .combine)
            HStack(spacing: Space.s) {
                Button { Task { await allow(r) } } label: { Working(busy: working == r.id, title: "Allow") }
                    .buttonStyle(.filled)
                Button("Refuse") { Task { await refuse(r) } }
                    .buttonStyle(.outlined)
            }
            .disabled(working != nil)
        }
        .padding(.vertical, Space.m)
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
                let n = await Self.allowPermitted(core: core, id: id, requests: list)
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

    private func allow(_ r: ChannelRequest) async {
        working = r.id
        defer { working = nil }
        do {
            try await core.run { [id] in try $0.channelAllow(id: id, requestId: r.id, actions: r.actions) }
            notice = "\(r.name) can \(Self.words(r.actions))."
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
