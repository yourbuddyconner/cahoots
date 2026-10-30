import Foundation

/// The one `Phone`, and the one serial queue every call to it goes through.
///
/// `Phone` holds a lock across its HTTP and signer callbacks, and a signature can wait on a
/// Face ID prompt, so no call may run on the main thread. Views ask through `run`, which hops to
/// the queue and back.
final class Core: @unchecked Sendable {
    let phone: Phone
    let signer: EnclaveSigner
    let store: PhoneStore
    private let queue: DispatchQueue

    private init(phone: Phone, signer: EnclaveSigner, store: PhoneStore, queue: DispatchQueue) {
        self.phone = phone
        self.signer = signer
        self.store = store
        self.queue = queue
    }

    static func open(store: PhoneStore) async throws -> Core {
        let queue = DispatchQueue(label: "network.cahoots.phone", qos: .userInitiated)
        let signer = EnclaveSigner()
        return try await withCheckedThrowingContinuation { c in
            queue.async {
                c.resume(with: Result {
                    let phone = try Phone.open(dir: store.directory.path, signer: signer, http: UrlSessionHttp())
                    store.relax()
                    return Core(phone: phone, signer: signer, store: store, queue: queue)
                })
            }
        }
    }

    /// Run `f` on the phone's queue.
    func run<T>(_ f: @escaping (Phone) throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { c in
            queue.async {
                c.resume(with: Result { try f(self.phone) })
            }
        }
    }

    /// Decode a camera frame. `decode_frame` reads no state and takes no lock, so the camera's
    /// own queue calls it directly rather than waiting behind a relay request.
    func decodeFrame(width: Int, height: Int, gray: Data) -> String? {
        dispatchPrecondition(condition: .notOnQueue(.main))
        return phone.decodeFrame(width: UInt32(width), height: UInt32(height), gray: gray)
    }

    /// The seat key a ceremony made, under `cahoots:seat:<ceremony id>`, for when it is thrown away.
    func forgetCeremonyKey(_ ceremony: String) {
        signer.deleteKey(label: "cahoots:seat:\(ceremony)")
    }
}

/// The words a person sees for an error.
func plain(_ error: Error) -> String {
    switch error {
    case PhoneError.Locked:
        return "This needs Face ID or your passcode."
    case let PhoneError.Network(m), let PhoneError.Storage(m), let PhoneError.NotFound(m),
         let PhoneError.BadCode(m), let PhoneError.Ceremony(m), let PhoneError.Chat(m),
         let PhoneError.Signer(m):
        return m
    default:
        return "Something went wrong."
    }
}

extension Data {
    var seats: [UInt8] { [UInt8](self) }
}
