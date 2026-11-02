import CoreNFC
import Foundation

/// The phone crate's `Nfc`: one `NFCTagReaderSession` per tap.
///
/// The crate calls it from the phone's queue while holding its lock, so every call blocks that
/// queue until Core NFC answers, and nothing here waits for the main thread: the reader's
/// delegate runs on a queue of its own. `begin` returns once a chip is connected, `transceive`
/// sends one APDU and returns its data with SW1 and SW2, `end` closes the reader sheet.
final class NfcTransport: NSObject, Nfc, @unchecked Sendable {
    /// How long a tap waits for a chip. Core NFC itself ends a session after 60 seconds.
    static let findTimeout: TimeInterval = 30
    /// How long one APDU may take before the tap is given up.
    static let commandTimeout: TimeInterval = 5

    private let queue = DispatchQueue(label: "network.cahoots.nfc")
    private let lock = NSLock()
    private var session: NFCTagReaderSession?
    private var tag: NFCISO7816Tag?
    /// Signalled when a chip is connected or the session ends without one.
    private var found: DispatchSemaphore?
    /// Why the session ended, if it did.
    private var failure: PhoneError?

    func begin(prompt: String) throws {
        guard NFCTagReaderSession.readingAvailable else {
            throw PhoneError.Nfc(message: "This phone cannot read NFC tags.")
        }
        let ready = DispatchSemaphore(value: 0)
        guard let s = NFCTagReaderSession(pollingOption: [.iso14443], delegate: self, queue: queue) else {
            throw PhoneError.Nfc(message: "iOS would not start the NFC reader. Try again.")
        }
        locked {
            session = s
            tag = nil
            failure = nil
            found = ready
        }
        s.alertMessage = prompt
        s.begin()
        if ready.wait(timeout: .now() + Self.findTimeout) == .timedOut {
            s.invalidate(errorMessage: "No card found.")
            clear()
            throw PhoneError.NoCard
        }
        let (connected, why) = locked { (tag != nil, failure) }
        if !connected {
            clear()
            throw why ?? PhoneError.NoCard
        }
    }

    func transceive(apdu: Data) throws -> Data {
        let (t, why) = locked { (tag, failure) }
        guard let t else { throw why ?? PhoneError.NoCard }
        guard let command = NFCISO7816APDU(data: apdu) else {
            throw PhoneError.Nfc(message: "Cahoots made a command the reader cannot send.")
        }
        let done = DispatchSemaphore(value: 0)
        let box = Box()
        t.sendCommand(apdu: command) { data, sw1, sw2, error in
            if let error {
                box.result = .failure(Self.mapped(error))
            } else {
                var answer = data
                answer.append(sw1)
                answer.append(sw2)
                box.result = .success(answer)
            }
            done.signal()
        }
        if done.wait(timeout: .now() + Self.commandTimeout) == .timedOut {
            throw PhoneError.Nfc(message: "The card stopped answering. Hold the phone still and try again.")
        }
        return try box.result.get()
    }

    func end(message: String, failed: Bool) throws {
        guard let s = locked({ session }) else { return }
        if failed {
            s.invalidate(errorMessage: message.isEmpty ? "Something went wrong. Try again." : message)
        } else {
            if !message.isEmpty { s.alertMessage = message }
            s.invalidate()
        }
        clear()
    }

    private func clear() {
        locked {
            session = nil
            tag = nil
            found = nil
        }
    }

    private func locked<T>(_ f: () -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return f()
    }

    /// The chip went quiet in the middle of a tap: the phone moved, or the badge's tap window
    /// closed under it.
    static let movedAway = "The card moved away. Hold the phone to it until it says done."

    /// A tap error in the crate's words: the person cancelling or no chip in time is `NoCard`.
    private static func mapped(_ error: Error) -> PhoneError {
        guard let e = error as? NFCReaderError else {
            return .Nfc(message: "The tap did not work: \(error.localizedDescription)")
        }
        switch e.code {
        case .readerSessionInvalidationErrorUserCanceled, .readerSessionInvalidationErrorSessionTimeout,
             .readerSessionInvalidationErrorSessionTerminatedUnexpectedly:
            return .NoCard
        case .readerSessionInvalidationErrorSystemIsBusy:
            return .Nfc(message: "The NFC reader is busy. Try again in a moment.")
        case .readerTransceiveErrorTagConnectionLost, .readerTransceiveErrorTagNotConnected:
            return .Nfc(message: movedAway)
        default:
            return .Nfc(message: "The tap did not work: \(e.localizedDescription)")
        }
    }

    /// The answer of one command, written by Core NFC's completion handler.
    private final class Box: @unchecked Sendable {
        var result: Result<Data, PhoneError> = .failure(.NoCard)
    }
}

extension NfcTransport: NFCTagReaderSessionDelegate {
    func tagReaderSessionDidBecomeActive(_ session: NFCTagReaderSession) {}

    func tagReaderSession(_ session: NFCTagReaderSession, didInvalidateWithError error: Error) {
        let ready: DispatchSemaphore? = locked {
            guard self.session === session else { return nil }
            failure = Self.mapped(error)
            tag = nil
            let r = found
            found = nil
            return r
        }
        ready?.signal()
    }

    func tagReaderSession(_ session: NFCTagReaderSession, didDetect tags: [NFCTag]) {
        guard let first = tags.first, case let .iso7816(chip) = first else {
            session.alertMessage = "That is not a badge’s chip. Hold the phone to the badge."
            session.restartPolling()
            return
        }
        session.connect(to: first) { error in
            if error != nil {
                session.restartPolling()
                return
            }
            let ready: DispatchSemaphore? = self.locked {
                guard self.session === session else { return nil }
                self.tag = chip
                let r = self.found
                self.found = nil
                return r
            }
            ready?.signal()
        }
    }
}
