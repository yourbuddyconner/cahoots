import CoreNFC
import SwiftUI

/// A bench check: hold the phone to a badge's chip and see what the chip says over NFC.
///
/// It finds a tag, selects the SE05x applet by its id, asks the applet for its version and asks
/// it for sixteen random bytes. Nothing it sends creates, changes or uses a key. Every step and
/// every error is written to a log the person can share.
struct NfcDiagnosticsView: View {
    @StateObject private var probe = NfcProbe()

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                Text("Hold the phone to a badge’s chip. The app selects the chip’s applet, asks for its version and asks for random bytes. It does not touch any key.")
                    .font(.subheadline).foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
                if !probe.available {
                    StatusLine(text: "This device cannot read NFC tags.")
                }
                Button(probe.running ? "Looking for a chip…" : "Hold the phone to the chip") { probe.start() }
                    .buttonStyle(FilledButtonStyle())
                    .disabled(!probe.available || probe.running)
                    .accessibilityIdentifier("nfc-start")
                if !probe.lines.isEmpty {
                    PaperSection(title: "What happened") {
                        VStack(alignment: .leading, spacing: Space.s) {
                            ForEach(probe.lines) { line in
                                HStack(alignment: .firstTextBaseline, spacing: Space.s) {
                                    if line.problem {
                                        Image(systemName: "exclamationmark.circle")
                                            .foregroundStyle(Color.ink)
                                            .accessibilityHidden(true)
                                    }
                                    Text(line.text)
                                        .font(.footnote.monospaced())
                                        .foregroundStyle(line.problem ? Color.ink : Color.muted)
                                        .fixedSize(horizontal: false, vertical: true)
                                        .textSelection(.enabled)
                                }
                            }
                        }
                        .padding(.vertical, Space.m)
                    }
                }
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .paperBackground()
        .navigationTitle("NFC check")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if !probe.lines.isEmpty {
                ToolbarItem(placement: .primaryAction) { ShareLink(item: probe.transcript) }
            }
        }
    }
}

struct NfcLogLine: Identifiable {
    let id = UUID()
    let text: String
    let problem: Bool
}

/// Runs one NFC reader session against an SE05x and logs each step.
@MainActor
final class NfcProbe: NSObject, ObservableObject {
    @Published var lines: [NfcLogLine] = []
    @Published var running = false

    /// The SE05x IoT applet, the same id the badge selects over I2C.
    static let appletId = Data([0xA0, 0x00, 0x00, 0x03, 0x96, 0x54, 0x53, 0x00,
                                0x00, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00])

    private var session: NFCTagReaderSession?
    private var closedByUs = false

    var available: Bool { NFCTagReaderSession.readingAvailable }

    var transcript: String {
        lines.map { ($0.problem ? "! " : "  ") + $0.text }.joined(separator: "\n")
    }

    func start() {
        lines = []
        closedByUs = false
        guard available else {
            add("This device cannot read NFC tags.", problem: true)
            return
        }
        guard let s = NFCTagReaderSession(pollingOption: [.iso14443], delegate: self, queue: nil) else {
            add("iOS refused to start an NFC session.", problem: true)
            return
        }
        s.alertMessage = "Hold the phone to the chip."
        session = s
        running = true
        add("Reader starting. Hold the top of the phone flat over the antenna.")
        s.begin()
    }

    private func add(_ text: String, problem: Bool = false) {
        lines.append(NfcLogLine(text: text, problem: problem))
    }

    private func finish(_ session: NFCTagReaderSession, _ message: String, problem: Bool = false) {
        add(message, problem: problem)
        closedByUs = true
        if problem {
            session.invalidate(errorMessage: message)
        } else {
            session.alertMessage = message
            session.invalidate()
        }
        running = false
    }

    private func handle(_ tags: [NFCTag], in session: NFCTagReaderSession) async {
        guard let tag = tags.first else { return }
        if tags.count > 1 { add("\(tags.count) tags in the field. Using the first.") }
        do {
            try await session.connect(to: tag)
        } catch {
            finish(session, "Could not connect to the tag: \(nfcText(error))", problem: true)
            return
        }
        switch tag {
        case let .iso7816(t):
            add("ISO 7816 tag found. UID \(hex(t.identifier)).")
            if !t.historicalBytes.isEmptyOrNil { add("Historical bytes \(hex(t.historicalBytes ?? Data())).") }
            if !t.initialSelectedAID.isEmpty { add("iOS selected applet \(t.initialSelectedAID) on discovery.") }
            await probeApplet(t, in: session)
        case let .miFare(t):
            finish(session, "MIFARE tag found (family \(t.mifareFamily.rawValue)), UID \(hex(t.identifier)). It does not speak ISO 7816, so no commands were sent.", problem: true)
        case let .iso15693(t):
            finish(session, "ISO 15693 tag found, UID \(hex(t.identifier)). That is a plain NFC tag, not the chip.", problem: true)
        case .feliCa:
            finish(session, "FeliCa tag found. Not the chip.", problem: true)
        @unknown default:
            finish(session, "A tag of an unknown kind was found.", problem: true)
        }
    }

    private func probeApplet(_ tag: NFCISO7816Tag, in session: NFCTagReaderSession) async {
        // SELECT by AID: 00 A4 04 00 10 <aid> 00
        let select = NFCISO7816APDU(instructionClass: 0x00, instructionCode: 0xA4, p1Parameter: 0x04,
                                    p2Parameter: 0x00, data: Self.appletId, expectedResponseLength: 256)
        guard let selected = await send(select, "SELECT applet", tag, session) else { return }
        if !selected.isEmpty { add("Applet answered SELECT with \(hex(selected)).") }

        // GetVersion: 80 04 00 20 00. The answer is TLV 41 with seven bytes.
        let version = NFCISO7816APDU(instructionClass: 0x80, instructionCode: 0x04, p1Parameter: 0x00,
                                     p2Parameter: 0x20, data: Data(), expectedResponseLength: 256)
        guard let v = await send(version, "GetVersion", tag, session) else { return }
        if let info = tlv41(v), info.count >= 7 {
            add("Applet version \(info[0]).\(info[1]).\(info[2]), config \(hex(info[3..<5])), secure box \(hex(info[5..<7])).")
        } else {
            add("GetVersion answered \(hex(v)), which is not the expected TLV.", problem: true)
        }

        // GetRandom 16: 80 04 00 49 04 41 02 00 10 00
        let random = NFCISO7816APDU(instructionClass: 0x80, instructionCode: 0x04, p1Parameter: 0x00,
                                    p2Parameter: 0x49, data: Data([0x41, 0x02, 0x00, 0x10]),
                                    expectedResponseLength: 256)
        guard let r = await send(random, "GetRandom 16", tag, session) else { return }
        if let bytes = tlv41(r), bytes.count == 16 {
            add("Random bytes \(hex(bytes)).")
        } else {
            add("GetRandom answered \(hex(r)), which is not sixteen bytes in a TLV.", problem: true)
        }

        finish(session, "The chip answers over NFC.")
    }

    /// Sends one APDU and logs the status word. Returns the data on 90 00, or nil after ending
    /// the session on any other status or a transport error.
    private func send(_ apdu: NFCISO7816APDU, _ name: String, _ tag: NFCISO7816Tag,
                      _ session: NFCTagReaderSession) async -> Data? {
        do {
            let (data, sw1, sw2) = try await tag.sendCommand(apdu: apdu)
            let sw = String(format: "%02X%02X", sw1, sw2)
            if sw1 == 0x90, sw2 == 0x00 {
                add("\(name): 90 00, \(data.count) bytes.")
                return data
            }
            finish(session, "\(name) failed with \(sw) (\(statusText(sw1, sw2))).", problem: true)
            return nil
        } catch {
            finish(session, "\(name) did not get through: \(nfcText(error))", problem: true)
            return nil
        }
    }

    private func tlv41(_ d: Data) -> Data? {
        let b = [UInt8](d)
        guard b.count >= 2, b[0] == 0x41 else { return nil }
        var i = 1
        var len = Int(b[i]); i += 1
        if len == 0x81, b.count > i { len = Int(b[i]); i += 1 }
        else if len == 0x82, b.count > i + 1 { len = Int(b[i]) << 8 | Int(b[i + 1]); i += 2 }
        guard b.count >= i + len else { return nil }
        return Data(b[i..<(i + len)])
    }

    private func statusText(_ sw1: UInt8, _ sw2: UInt8) -> String {
        switch (sw1, sw2) {
        case (0x6A, 0x82): return "applet not found"
        case (0x6E, 0x00): return "class not supported"
        case (0x6D, 0x00): return "instruction not supported"
        case (0x69, 0x82): return "security status not satisfied"
        case (0x69, 0x85): return "conditions not satisfied"
        case (0x67, 0x00): return "wrong length"
        case (0x6A, 0x86): return "wrong parameters"
        default: return "unexpected status"
        }
    }

    private func nfcText(_ error: Error) -> String {
        guard let e = error as? NFCReaderError else { return error.localizedDescription }
        switch e.code {
        case .readerSessionInvalidationErrorUserCanceled: return "cancelled"
        case .readerSessionInvalidationErrorSessionTimeout: return "no chip within the time limit"
        case .readerSessionInvalidationErrorSystemIsBusy: return "the NFC reader is busy"
        case .readerTransceiveErrorTagConnectionLost: return "the chip left the field"
        case .readerTransceiveErrorTagResponseError: return "the chip answered with an error frame"
        case .readerTransceiveErrorRetryExceeded: return "too many retries"
        case .readerTransceiveErrorTagNotConnected: return "the tag is not connected"
        case .readerTransceiveErrorPacketTooLong: return "the command is too long"
        default: return "\(e.localizedDescription) (code \(e.code.rawValue))"
        }
    }

    private func hex(_ d: Data) -> String { d.map { String(format: "%02X", $0) }.joined(separator: " ") }
}

extension NfcProbe: NFCTagReaderSessionDelegate {
    nonisolated func tagReaderSessionDidBecomeActive(_ session: NFCTagReaderSession) {
        Task { @MainActor in self.add("Reader is on. Looking for a chip.") }
    }

    nonisolated func tagReaderSession(_ session: NFCTagReaderSession, didInvalidateWithError error: Error) {
        Task { @MainActor in
            if !self.closedByUs {
                self.add("Session ended: \(self.nfcText(error)).", problem: true)
            }
            self.running = false
            self.session = nil
        }
    }

    nonisolated func tagReaderSession(_ session: NFCTagReaderSession, didDetect tags: [NFCTag]) {
        Task { @MainActor in await self.handle(tags, in: session) }
    }
}

private extension Optional where Wrapped == Data {
    var isEmptyOrNil: Bool { self?.isEmpty ?? true }
}
