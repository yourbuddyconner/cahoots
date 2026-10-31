import Foundation

/// Where the phone crate keeps its records, and the iOS protection on them.
///
/// Everything is under Application Support/cahoots, left out of every backup: a restored phone
/// has no enclave keys, and a pad without its seat key is worse than nothing. The circles are
/// unreadable while the phone is locked (complete protection). The relay cursor, the per-circle
/// settings and `phone.cbor` hold nothing secret, so they stay readable after the first unlock
/// and background refresh can open them.
struct PhoneStore {
    let directory: URL

    var circles: URL { directory.appendingPathComponent("circles", isDirectory: true) }

    /// `root` overrides the directory, for tests.
    init(root: URL? = nil) throws {
        let fm = FileManager.default
        if let root {
            directory = root
        } else {
            let support = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                     appropriateFor: nil, create: true)
            directory = support.appendingPathComponent(Self.folderName, isDirectory: true)
        }
        try fm.createDirectory(at: circles, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        var dir = directory
        try dir.setResourceValues(values)
        // New files take the protection of the folder they are made in. The phone crate writes
        // every file by a temporary file and a rename inside the circle's folder.
        try fm.setAttributes([.protectionKey: FileProtectionType.complete], ofItemAtPath: circles.path)
        relax()
    }

    /// The folder's name. A debug build takes `-cahoots-data <name>` so UI tests start empty.
    private static var folderName: String {
        #if DEBUG
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-cahoots-data"), i + 1 < args.count {
            return args[i + 1]
        }
        #endif
        return "cahoots"
    }

    /// How many circle folders there are, readable or not.
    var circleCount: Int {
        let ids = (try? FileManager.default.contentsOfDirectory(atPath: circles.path)) ?? []
        return ids.filter { !$0.hasPrefix(".") }.count
    }

    /// Private labels live beside the circle, inherit its protection, and leave with it.
    func seatNames(circle: String) throws -> [String: String] {
        let url = circles.appendingPathComponent(circle).appendingPathComponent("seat-names.json")
        guard FileManager.default.fileExists(atPath: url.path) else { return [:] }
        return try JSONDecoder().decode([String: String].self, from: Data(contentsOf: url))
    }

    func setSeatName(_ name: String, circle: String, seat: UInt8) throws {
        var names = try seatNames(circle: circle)
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        names[String(seat)] = trimmed.isEmpty ? nil : trimmed
        let url = circles.appendingPathComponent(circle).appendingPathComponent("seat-names.json")
        try JSONEncoder().encode(names).write(to: url, options: [.atomic, .completeFileProtection])
    }

    /// Give the files that hold nothing secret protection until first unlock. The phone crate
    /// rewrites them on every poll, and a rewritten file takes the folder's complete protection,
    /// so call this after each poll.
    func relax() {
        let fm = FileManager.default
        let open: [FileAttributeKey: Any] = [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
        var files = [directory.appendingPathComponent("phone.cbor")]
        let ids = (try? fm.contentsOfDirectory(atPath: circles.path)) ?? []
        for id in ids where !id.hasPrefix(".") {
            let dir = circles.appendingPathComponent(id, isDirectory: true)
            files.append(dir.appendingPathComponent("cursor.cbor"))
            files.append(dir.appendingPathComponent("settings.cbor"))
        }
        for f in files where fm.fileExists(atPath: f.path) {
            try? fm.setAttributes(open, ofItemAtPath: f.path)
        }
    }
}
