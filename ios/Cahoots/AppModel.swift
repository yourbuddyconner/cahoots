import SwiftUI
import UIKit
import UserNotifications

enum Route: Hashable {
    case newCircle
    case join
    case ceremony(String)
    case chat(String)
    case people(String)
    case hello(String, UInt8)
    case settings
    /// A channel on this phone, by id.
    case channel(String)
    /// A channel link opened from outside, the whole URL.
    case joinChannel(String)
    case channelMembers(String)
    case channelRequests(String)
}

/// What the whole app shares: the opened phone, the list of circles, and where the person is.
@MainActor
final class AppModel: ObservableObject {
    @Published var core: Core?
    @Published var openError: String?
    @Published var circles: [CircleSummary] = []
    /// Every channel this phone owns or reads.
    @Published var channels: [ChannelSummary] = []
    /// A channel just started here: its screen opens with the share sheet.
    @Published var shareOnOpen: String?
    /// A link that arrived before the phone opened, followed once it has.
    private var waitingLink: URL?
    /// Each circle's faces, drawn, in seat order, by circle id.
    @Published var faces: [String: [Data]] = [:]
    @Published private(set) var seatNames: [String: [String: String]] = [:]
    @Published var path: [Route] = [] {
        didSet {
            // A link held while a ceremony was open goes once the ceremony leaves the screen.
            if waitingLink != nil { Task { @MainActor in self.followWaitingLink() } }
        }
    }
    /// Shown once at launch when a ceremony was open when the app closed. A link that arrived
    /// meanwhile is followed once the alert is dismissed.
    @Published var lostCeremony: String? {
        didSet {
            if lostCeremony == nil, oldValue != nil, waitingLink != nil {
                Task { @MainActor in self.followWaitingLink() }
            }
        }
    }
    let ceremonies = OpenCeremonies()

    private var opening: Task<Core?, Never>?

    /// Open the phone once. Callers that arrive while it opens wait for the same open.
    @discardableResult
    func open() async -> Core? {
        if let core { return core }
        if let opening { return await opening.value }
        let t = Task { () -> Core? in
            do {
                let store = try PhoneStore()
                let core = try await Core.open(store: store)
                self.core = core
                self.openError = nil
                let lost = self.ceremonies.takeLost()
                for label in lost { core.signer.deleteKey(label: label) }
                if !lost.isEmpty { self.lostCeremony = OpenCeremonies.lostMessage }
                return core
            } catch {
                self.openError = plain(error)
                return nil
            }
        }
        opening = t
        let c = await t.value
        opening = nil
        if c != nil {
            await refresh()
            followWaitingLink()
        }
        return c
    }

    /// Read the list of circles. While the phone is locked the circles cannot be read, and the
    /// list (and the badge) stay as they were rather than look empty. False when not read.
    @discardableResult
    func refresh() async -> Bool {
        guard let core, UIApplication.shared.isProtectedDataAvailable else { return false }
        guard let list = try? await core.run({ $0.circles() }) else { return false }
        circles = list
        if let chs = try? await core.run({ $0.channels() }) { channels = chs }
        seatNames = Dictionary(uniqueKeysWithValues: list.map {
            ($0.id, (try? core.store.seatNames(circle: $0.id)) ?? [:])
        })
        faces = (try? await core.run { phone in
            Dictionary(uniqueKeysWithValues: list.map { c in
                (c.id, (0..<c.members).compactMap { phone.seatFace(circle: c.id, seat: $0) })
            })
        }) ?? faces
        return true
    }

    func circle(_ id: String) -> CircleSummary? {
        circles.first { $0.id == id }
    }

    func channel(_ id: String) -> ChannelSummary? {
        channels.first { $0.id == id }
    }

    /// What to do with a link from outside the app.
    enum LinkAction: Equatable {
        /// Not a link into the web app.
        case ignore
        /// Another web app link: Safari, which is where iOS sends a universal link the app
        /// opens itself.
        case safari
        /// The phone is still opening: follow it once it has.
        case queue
        /// A ceremony is on screen: follow it once the ceremony is done or left.
        case hold
        /// Push this route on the current path.
        case open(Route)
        /// A channel this phone already reads: take the link (a newer one replaces the old)
        /// and open the channel, with no join screen.
        case rejoin(id: String, link: String)
    }

    /// The decision for `url`, from what is on screen and on this phone. `ready` is whether
    /// the phone has opened. Pure, so the tests can read it.
    nonisolated static func linkAction(_ url: URL, ready: Bool, path: [Route],
                                       channels: [ChannelSummary]) -> LinkAction {
        guard url.scheme == "https", url.host == "cahoots.network",
              url.path == "/app" || url.path.hasPrefix("/app/") else { return .ignore }
        guard let id = channelLinkId(url), let link = canonicalLink(url) else { return .safari }
        guard ready else { return .queue }
        if path.contains(where: { if case .ceremony = $0 { return true } else { return false } }) {
            return .hold
        }
        guard let known = channels.first(where: { $0.id == id }) else { return .open(.joinChannel(link)) }
        if known.role == .owner { return .open(.channel(id)) }
        return .rejoin(id: id, link: link)
    }

    /// A link from outside the app: a universal link, or one tapped in another app.
    /// `https://cahoots.network/app/#/ch/<relay host>/<channel id>[/<secret>]` opens the channel
    /// (or its join screen); any other link into the web app goes to Safari.
    func openLink(_ url: URL) {
        switch Self.linkAction(url, ready: core != nil, path: path, channels: channels) {
        case .ignore:
            return
        case .safari:
            UIApplication.shared.open(url, options: [.universalLinksOnly: false])
        case .queue, .hold:
            waitingLink = url
        case let .open(route):
            dismissSheets()
            // A link delivered twice (a tap and a universal link) opens its screen once.
            if path.last != route { path.append(route) }
        case let .rejoin(id, link):
            dismissSheets()
            Task {
                // The same link changes nothing; a newer one replaces the old. Either way the
                // channel opens, and the join's error, if any, shows there on the next poll.
                _ = try? await core?.run { try $0.channelJoin(url: link) }
                await refresh()
                if path.last != .channel(id) { path.append(.channel(id)) }
            }
        }
    }

    /// A held link, once no ceremony is on screen and the "Ceremony lost" alert is gone: following
    /// it closes sheets, which would take the alert with it unread.
    private func followWaitingLink() {
        guard let link = waitingLink, core != nil, lostCeremony == nil,
              !path.contains(where: { if case .ceremony = $0 { return true } else { return false } })
        else { return }
        waitingLink = nil
        openLink(link)
    }

    /// Close whatever sheet is up, so the link's screen is the one in front.
    private func dismissSheets() {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        for window in scenes.flatMap(\.windows) where window.isKeyWindow {
            window.rootViewController?.presentedViewController?.dismiss(animated: true)
        }
    }

    /// The channel id in a link whose fragment is `/ch/<host>/<id>/<secret>`, or
    /// `/ch/<host>/<id>` for a closed channel's link without its secret. Nil for anything else.
    nonisolated static func channelLinkId(_ url: URL) -> String? {
        guard let fragment = url.fragment else { return nil }
        let parts = fragment.split(separator: "/", omittingEmptySubsequences: true).map(String.init)
        guard parts.count == 3 || parts.count == 4, parts[0] == "ch" else { return nil }
        return parts[2]
    }

    /// The link in the one form the phone crate reads: `https://cahoots.network/app/#<fragment>`,
    /// whatever the path's slash or a query string.
    nonisolated static func canonicalLink(_ url: URL) -> String? {
        guard let fragment = url.fragment, !fragment.isEmpty else { return nil }
        return "https://cahoots.network/app/#\(fragment)"
    }

    func seatName(circle: String, seat: UInt8) -> String {
        seatNames[circle]?[String(seat)] ?? "Seat \(seat + 1)"
    }

    func setSeatName(_ name: String, circle: String, seat: UInt8) throws {
        guard let core else { throw PhoneError.Storage(message: "Your circles are still opening. Try again.") }
        try core.store.setSeatName(name, circle: circle, seat: seat)
        seatNames[circle] = try core.store.seatNames(circle: circle)
    }

    /// Read every circle's relay, then show the unread count on the icon. Each circle gets at
    /// most 20 seconds, and a cancelled task stops between circles. The badge changes only after
    /// a pass in which every circle was read and polled.
    func pollAll() async -> Bool {
        guard let core = await open() else { return false }
        guard UIApplication.shared.isProtectedDataAvailable else { return false }
        let onDisk = core.store.circleCount
        guard let ids = await core.run(within: 20, { $0.circles().map(\.id) }) else { return false }
        var ok = ids.count == onDisk
        for id in ids {
            if Task.isCancelled { return false }
            guard await core.run(within: 20, { try $0.poll(circle: id) }) != nil else {
                ok = false
                continue
            }
        }
        // Channels: a dead link or a relay that cannot be reached leaves the circles' pass alone.
        let channelIds = (try? await core.run { $0.channels() })?.filter { !$0.dead }.map(\.id) ?? []
        for id in channelIds {
            if Task.isCancelled { return false }
            _ = await core.run(within: 20, { try $0.channelPoll(id: id) })
        }
        core.store.relax()
        if Task.isCancelled { return false }
        guard await refresh(), ok else { return false }
        await updateBadge()
        return true
    }

    func updateBadge() async {
        let unread = circles.reduce(0) { $0 + Int($1.unread) } + channels.reduce(0) { $0 + Int($1.unread) }
        try? await UNUserNotificationCenter.current().setBadgeCount(unread)
    }

    /// The icon badge needs permission. Asked once there is a circle to count for.
    func askForBadge() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.badge]) { _, _ in }
    }

    func startCeremony(name: String, members: UInt8, padLen: UInt32, throughRelay: Bool) async throws {
        guard let core else { return }
        let id = try await core.run {
            try $0.startCeremony(name: name, members: members, padLen: padLen, throughRelay: throughRelay)
        }
        ceremonies.record(id)
        path = [.ceremony(id)]
    }

    func joined(_ ceremony: String) {
        ceremonies.record(ceremony)
        path = [.ceremony(ceremony)]
    }

    func kept(_ circle: String) async {
        await refresh()
        askForBadge()
        path = [.chat(circle)]
    }

    func leave() {
        path = []
        Task { await refresh() }
    }

    /// Start a channel owned by `circle`, then open it with the share sheet.
    func startChannel(circle: String, name: String, preset: UInt8) async throws {
        guard let core else { return }
        let summary = try await core.run { try $0.channelStart(circle: circle, name: name, preset: preset) }
        await refresh()
        shareOnOpen = summary.id
        path = [.channel(summary.id)]
    }

    /// Remove a channel from this phone, and its keys with it.
    func deleteChannel(_ id: String) async throws {
        guard let core else { return }
        try await core.run { try $0.channelDelete(id: id) }
        path = []
        await refresh()
        await updateBadge()
    }

    /// Delete a circle from this phone, and its seat key with it.
    func delete(_ circle: String) async throws {
        guard let core else { return }
        let label = try await core.run { try $0.circleSeatLabel(circle: circle) }
        try await core.run { try $0.deleteCircle(circle: circle) }
        core.signer.deleteKey(label: label)
        path = []
        await refresh()
        await updateBadge()
    }
}
