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
}

/// What the whole app shares: the opened phone, the list of circles, and where the person is.
@MainActor
final class AppModel: ObservableObject {
    @Published var core: Core?
    @Published var openError: String?
    @Published var circles: [CircleSummary] = []
    @Published var path: [Route] = []
    /// Shown once at launch when a ceremony was open when the app closed.
    @Published var lostCeremony: String?
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
        if c != nil { await refresh() }
        return c
    }

    /// Read the list of circles. While the phone is locked the circles cannot be read, and the
    /// list (and the badge) stay as they were rather than look empty. False when not read.
    @discardableResult
    func refresh() async -> Bool {
        guard let core, UIApplication.shared.isProtectedDataAvailable else { return false }
        guard let list = try? await core.run({ $0.circles() }) else { return false }
        circles = list
        return true
    }

    func circle(_ id: String) -> CircleSummary? {
        circles.first { $0.id == id }
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
        core.store.relax()
        if Task.isCancelled { return false }
        guard await refresh(), ok else { return false }
        await updateBadge()
        return true
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
