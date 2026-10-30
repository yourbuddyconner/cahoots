import Foundation

/// The ceremonies this phone has open, kept in UserDefaults. A ceremony lives only in the phone
/// crate's memory, so one still recorded when the app launches was lost when the app closed: the
/// person is told once, and the seat key made for it is deleted from the keychain.
struct OpenCeremonies {
    static let key = "cahoots.openCeremonies"
    let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// The `Signer` label of the seat key a ceremony made.
    static func keyLabel(_ ceremony: String) -> String { "cahoots:seat:\(ceremony)" }

    var ids: [String] { defaults.stringArray(forKey: Self.key) ?? [] }

    func record(_ ceremony: String) {
        var all = ids
        if !all.contains(ceremony) { all.append(ceremony) }
        defaults.set(all, forKey: Self.key)
    }

    /// The ceremony was kept or thrown away.
    func clear(_ ceremony: String) {
        let rest = ids.filter { $0 != ceremony }
        if rest.isEmpty { defaults.removeObject(forKey: Self.key) } else { defaults.set(rest, forKey: Self.key) }
    }

    /// At launch: every recorded ceremony is lost. Returns their seat key labels and forgets them.
    func takeLost() -> [String] {
        let lost = ids.map(Self.keyLabel)
        defaults.removeObject(forKey: Self.key)
        return lost
    }

    static let lostMessage = "This ceremony was lost when the app closed. Everyone starts over with a new code."
}
