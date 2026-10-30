import SwiftUI
import UserNotifications

enum Route: Hashable {
    case newCircle
    case join
    case ceremony(String)
    case chat(String)
    case people(String)
    case hello(String, UInt8)
    case settings
}

/// What the whole app shares: the opened phone, the list of circles, and where the person is.
@MainActor
final class AppModel: ObservableObject {
    @Published var core: Core?
    @Published var openError: String?
    @Published var circles: [CircleSummary] = []
    @Published var path: [Route] = []

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
                return core
            } catch {
                self.openError = plain(error)
                return nil
            }
        }
        opening = t
        let c = await t.value
        opening = nil
        if c != nil { await refresh() }
        return c
    }

    func refresh() async {
        guard let core else { return }
        if let list = try? await core.run({ $0.circles() }) {
            circles = list
        }
    }

    func circle(_ id: String) -> CircleSummary? {
        circles.first { $0.id == id }
    }

    /// Read every circle's relay, then show the unread count on the icon.
    func pollAll() async -> Bool {
        guard let core = await open() else { return false }
        var ok = true
        let ids = (try? await core.run { $0.circles().map(\.id) }) ?? []
        for id in ids {
            do {
                _ = try await core.run { try $0.poll(circle: id) }
            } catch {
                ok = false
            }
        }
        core.store.relax()
        await refresh()
        await updateBadge()
        return ok
    }

    func updateBadge() async {
        let unread = circles.reduce(0) { $0 + Int($1.unread) }
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
        path = [.ceremony(id)]
    }

    func joined(_ ceremony: String) {
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
