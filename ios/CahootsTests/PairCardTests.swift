import XCTest
@testable import Cahoots

/// The pairing screen's steps, with the two taps stood in for: the simulator has no NFC.
@MainActor
final class PairCardTests: XCTestCase {
    private final class FakePairing: CardPairing, @unchecked Sendable {
        var startError: Error?
        var finishErrors: [Error] = []
        var names: [String] = []

        func startPairing(name: String) async throws -> PairPreview {
            if let startError { throw startError }
            return PairPreview(badgeName: "Conner's badge", circles: "2 circles",
                               words: ["amber", "basil", "cedar", "delta", "eagle", "fabric"])
        }

        func finishPairing() async throws -> [String] {
            if !finishErrors.isEmpty { throw finishErrors.removeFirst() }
            return ["park", "lab"]
        }
    }

    func testTheWordsThenTheCirclesItBrought() async {
        let model = PairCardModel(pairing: FakePairing())
        XCTAssertEqual(model.step, .name)
        await model.start()
        guard case let .check(preview) = model.step else { return XCTFail("no words: \(model.step)") }
        XCTAssertEqual(preview.words.count, 6)
        XCTAssertEqual(preview.circles, "2 circles")
        await model.finish()
        XCTAssertEqual(model.step, .done(badge: "Conner's badge", circles: ["park", "lab"]))
        XCTAssertNil(model.error)
    }

    func testAProblemOnTheFirstTapStaysOnTheName() async {
        let fake = FakePairing()
        fake.startError = PhoneError.Ceremony(message: "Enter a name for this phone.")
        let model = PairCardModel(pairing: fake)
        await model.start()
        XCTAssertEqual(model.step, .name)
        XCTAssertEqual(model.error, "Enter a name for this phone.")
    }

    func testABadgeNotYetPressedKeepsTheWordsForAnotherTap() async {
        let fake = FakePairing()
        fake.finishErrors = [PhoneError.NotPaired]
        let model = PairCardModel(pairing: fake)
        await model.start()
        await model.finish()
        XCTAssertEqual(model.error, "Press both buttons on the badge first, then tap the card again.")
        guard case .check = model.step else { return XCTFail("the words went: \(model.step)") }
        await model.finish()
        XCTAssertEqual(model.step, .done(badge: "Conner's badge", circles: ["park", "lab"]))
        XCTAssertNil(model.error)
    }

    /// The crate refuses a second badge with its own error, and the screen says what to do
    /// about it under the sentence, whatever the sentence says.
    func testASecondBadgeKeepsTheWordsAndSaysWhatToDo() async {
        let fake = FakePairing()
        fake.finishErrors = [PhoneError.OtherBadge]
        let model = PairCardModel(pairing: fake)
        await model.start()
        await model.finish()
        XCTAssertEqual(model.error, "Forget this card first.")
        XCTAssertEqual(model.note, PairCardModel.note(after: PhoneError.OtherBadge))
        XCTAssertNotNil(model.note)
        guard case .check = model.step else { return XCTFail("the words went: \(model.step)") }
        model.startOver()
        XCTAssertNil(model.note)
    }

    func testStartOverForgetsTheWords() async {
        let model = PairCardModel(pairing: FakePairing())
        await model.start()
        model.startOver()
        XCTAssertEqual(model.step, .name)
        XCTAssertNil(model.error)
    }

    /// The real crate refuses an empty name before it taps.
    func testThePhoneAsksForAName() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let phone = try Phone.open(dir: dir.path, signer: EnclaveSigner(), http: UrlSessionHttp(), nfc: NfcTransport())
        XCTAssertThrowsError(try phone.pairCardStart(name: "  ")) { error in
            XCTAssertEqual(plain(error), "Enter a name for this phone.")
        }
    }

    private func summary(kind: String, needsTap: Bool = false) -> CircleSummary {
        CircleSummary(id: "c", name: "park", members: 3, mySeat: 1, hasRoot: kind != "card", unread: 0,
                      relay: "https://relay.cahoots.network", kind: kind, needsTap: needsTap)
    }

    func testACardCircleSaysSoInWords() {
        XCTAssertEqual(CircleRow.meta(summary(kind: "card"), others: "Grace", badge: "Conner's badge"),
                       "Card circle on Conner's badge. With Grace.")
        XCTAssertEqual(CircleRow.meta(summary(kind: "card", needsTap: true), others: nil, badge: "Conner's badge"),
                       "Card circle on Conner's badge. Three people. Tap the card.")
        XCTAssertEqual(CircleRow.meta(summary(kind: "seat"), others: "Grace", badge: nil),
                       "With Grace. You are seat 2.")
    }

    func testACardCirclesOwnSeatIsYourCard() {
        let badge = Member(seat: 1, name: "Conner's badge", faceSeed: Data(count: 32), hasCard: true,
                           device: 1, tier: 3, mine: true)
        XCTAssertEqual(PeopleView.about(badge, card: true), "Tier 3, a hardware key store, your card")
        XCTAssertEqual(PeopleView.about(badge), "Tier 3, a hardware key store, on a badge")
    }
}
