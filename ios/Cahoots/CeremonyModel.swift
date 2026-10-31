import SwiftUI

/// One ceremony in progress, as the phone crate reports it. Through the relay it ticks every two
/// seconds; with codes only it moves when a code is scanned. Either way, when step three asks
/// for this seat's signature it signs at once, which may ask for Face ID.
@MainActor
final class CeremonyModel: ObservableObject {
    let core: Core
    let id: String

    @Published private(set) var state: CeremonyState?
    /// Codes only: the frames of this seat's current code, drawn.
    @Published private(set) var frames: [QrMatrix] = []
    /// Through the relay, on the host: the one code everyone joining scans.
    @Published private(set) var offer: QrMatrix?
    /// Through the relay, on a joiner: the code to show the host, until the host has it.
    @Published private(set) var joinCode: QrMatrix?
    @Published var status: String?
    @Published var notice: String?
    @Published private(set) var signing = false
    /// Face ID was refused or cancelled. The person signs again with a button, not by a prompt
    /// that comes back every two seconds.
    @Published private(set) var signRefused = false
    /// The phone no longer holds this ceremony: the app closed, or it was thrown away.
    @Published private(set) var gone: String?
    /// Codes only: the others have scanned this seat's signature, so the words can be compared.
    @Published var handedOff = false

    private var framesKey = ""
    private var joinCodeText: String?

    init(core: Core, id: String) {
        self.core = core
        self.id = id
    }

    var host: Bool { state?.mySeat == 0 }

    var finished: Bool {
        guard let s = state else { return gone != nil }
        return gone != nil || s.seatLost || s.phase == .complete || s.phase == .aborted
    }

    func load() async {
        do {
            let s = try await core.run { [id] in try $0.ceremonyState(id: id) }
            if s.throughRelay && s.mySeat == 0 {
                offer = try await core.run { [id] in try $0.qrMatrix(text: try $0.offerCode(id: id)) }
            }
            await apply(s)
        } catch {
            fail(error)
        }
    }

    /// Through the relay: tick every two seconds until the ceremony ends.
    func run() async {
        while !Task.isCancelled, !finished {
            await tick()
            if finished { break }
            try? await Task.sleep(for: .seconds(2))
        }
    }

    private func tick() async {
        do {
            var s = try await core.run { [id] in try $0.ceremonyTick(id: id) }
            if s.needsSignature, !signRefused, let signed = try await sign() {
                s = signed
                s = try await core.run { [id] in try $0.ceremonyTick(id: id) }
            }
            if relayTrouble { status = nil; relayTrouble = false }
            await apply(s)
        } catch {
            fail(error)
        }
    }

    /// The status shows a relay problem, cleared by the next tick that goes through.
    private var relayTrouble = false

    private func fail(_ error: Error) {
        switch error {
        case PhoneError.NotFound(let m):
            gone = m
        case PhoneError.Network(let m):
            // No answer at all: the tick tries again. An answer with an error: its own words.
            status = m == UrlSessionHttp.unreachable ? "\(m) Trying again." : m
            relayTrouble = true
        default:
            status = plain(error)
        }
        // A seat lost to another device answers every tick with an error. The state says why.
        Task {
            if let s = try? await core.run({ [id] in try $0.ceremonyState(id: id) }) {
                state = s
            }
        }
    }

    /// Sign the manifest with this seat's key. Face ID may ask first.
    private func sign() async throws -> CeremonyState? {
        signing = true
        defer { signing = false }
        do {
            return try await core.run { [id] in try $0.ceremonySign(id: id) }
        } catch PhoneError.Locked {
            signRefused = true
            status = "This needs Face ID or your passcode."
            return nil
        }
    }

    /// Sign again after Face ID was refused.
    func retrySign() async {
        signRefused = false
        status = nil
        do {
            if let s = try await sign() {
                await apply(s)
                if s.throughRelay { await tick() }
            }
        } catch {
            fail(error)
        }
    }

    private func apply(_ s: CeremonyState) async {
        state = s
        if s.throughRelay {
            if s.myJoinCode != joinCodeText {
                joinCodeText = s.myJoinCode
                if let t = s.myJoinCode {
                    joinCode = try? await core.run { try $0.qrMatrix(text: t) }
                } else {
                    joinCode = nil
                }
            }
            return
        }
        // Codes only: the code to show changes with the step, and again once signed.
        let key = "\(s.phase)|\(s.needsSignature)"
        if key != framesKey, s.phase != .aborted {
            framesKey = key
            do {
                frames = try await core.run { [id] p in
                    try p.ceremonyFrames(id: id).map { try p.qrMatrix(text: $0) }
                }
            } catch {
                status = plain(error)
            }
        }
    }

    /// A code from the camera, or pasted. Through the relay only the host scans, and only a
    /// joiner's code. With codes only, every code of the ceremony.
    func scan(_ text: String, pasted: Bool) async {
        guard let before = state else { return }
        if before.throughRelay && !pasted && !text.hasPrefix("cahoots:joined:v1:") { return }
        do {
            var s = try await core.run { [id] in try $0.ceremonyScan(id: id, text: text) }
            if s.needsSignature, !signRefused, let signed = try await sign() {
                s = signed
            }
            let newly = Set(s.scanned.seats).subtracting(before.scanned.seats)
            if let seat = newly.first {
                notice = "Scanned seat \(seat + 1)"
            }
            if pasted || status != nil { status = nil }
            await apply(s)
        } catch PhoneError.BadCode(let m) {
            // The camera sees many things that are not codes. Only a pasted code is worth a word.
            if pasted { status = m }
        } catch {
            fail(error)
        }
    }

    /// The host lets the ceremony go on once everyone is scanned.
    func proceed() async {
        do {
            let s = try await core.run { [id] in try $0.ceremonyContinue(id: id) }
            await apply(s)
        } catch {
            fail(error)
        }
    }

    /// The words match: write the circle. Returns its id.
    func keep() async -> String? {
        do {
            let circle = try await core.run { [id] in try $0.ceremonyKeep(id: id) }
            OpenCeremonies().clear(id)
            return circle
        } catch {
            status = plain(error)
            return nil
        }
    }

    /// Throw the ceremony and its seat key away. Nothing of it was written.
    func discard() async {
        _ = try? await core.run { [id] in $0.ceremonyDiscard(id: id) }
        core.forgetCeremonyKey(id)
        OpenCeremonies().clear(id)
    }
}
