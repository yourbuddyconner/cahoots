import BackgroundTasks
import Foundation

/// iOS wakes the app now and then, when it decides to, to read each circle's relay and set the
/// count on the icon. Nothing else: no notifications beyond the badge.
enum BackgroundRefresh {
    static let identifier = "network.cahoots.refresh"

    /// Call once, before the app finishes launching.
    @MainActor
    static func register(_ model: AppModel) {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: nil) { task in
            schedule()
            let work = Task { @MainActor in
                let ok = await model.pollAll()
                task.setTaskCompleted(success: ok)
            }
            task.expirationHandler = { work.cancel() }
        }
    }

    /// Ask for the next wake, at most every fifteen minutes. iOS may wait much longer.
    static func schedule() {
        let request = BGAppRefreshTaskRequest(identifier: identifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }
}
