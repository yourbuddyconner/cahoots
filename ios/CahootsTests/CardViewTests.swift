import XCTest
@testable import Cahoots

/// The Card screen's words: what a tap would do now, what one did, and the lines other screens
/// show about the card.
@MainActor
final class CardViewTests: XCTestCase {
    private func status(keys: UInt32 = 0, bumps: UInt32 = 0, low: [WordsLeft] = [],
                        lastTap: UInt64? = nil) -> CardStatus {
        CardStatus(paired: true, badge: "Conner's badge", circles: 2, keysWanted: keys,
                   bumpsToPost: bumps, wordsLow: low, lastTap: lastTap)
    }

    private func name(_ circle: String, _ seat: UInt8) -> String {
        circle == "c" && seat == 1 ? "Ada" : "Seat \(seat + 1)"
    }

    func testNothingWaitingIsUpToDate() {
        XCTAssertEqual(CardView.summary(status(), name: name), ["up to date"])
    }

    func testEachThingATapWouldDoHasALine() {
        let s = status(keys: 2, bumps: 1, low: [WordsLeft(circle: "c", seat: 1, left: 2)])
        XCTAssertEqual(CardView.summary(s, name: name),
                       ["2 circles need a key", "1 bump to post", "hello words: 2 left for Ada"])
    }

    func testOneOfEachIsSingularAndEverySeatLowHasItsOwnLine() {
        let s = status(keys: 1, bumps: 3, low: [WordsLeft(circle: "c", seat: 1, left: 1),
                                                WordsLeft(circle: "d", seat: 2, left: 3)])
        XCTAssertEqual(CardView.summary(s, name: name),
                       ["1 circle needs a key", "3 bumps to post", "hello words: 1 left for Ada",
                        "hello words: 3 left for Seat 3"])
    }

    func testWhatATapDidInOneSentence() {
        XCTAssertEqual(CardView.result(TapReport(keys: 2, bumps: 1, words: 16)),
                       "Fetched 2 keys and 16 hello words, and passed on 1 bump.")
        XCTAssertEqual(CardView.result(TapReport(keys: 0, bumps: 0, words: 0)), "Nothing was waiting.")
        XCTAssertEqual(CardView.result(TapReport(keys: 1, bumps: 0, words: 0)), "Fetched 1 key.")
        XCTAssertEqual(CardView.result(TapReport(keys: 0, bumps: 0, words: 1)), "Fetched 1 hello word.")
        XCTAssertEqual(CardView.result(TapReport(keys: 0, bumps: 2, words: 0)), "Passed on 2 bumps.")
        XCTAssertEqual(CardView.result(TapReport(keys: 0, bumps: 1, words: 8)),
                       "Fetched 8 hello words, and passed on 1 bump.")
    }

    func testTheLastTapIsNeverOrHowLongAgo() {
        XCTAssertEqual(CardView.lastTap(nil), "last tap: never")
        let now = Date(timeIntervalSince1970: 1_000_000)
        let ago = CardView.lastTap(1_000_000 - 120, now: now)
        XCTAssertTrue(ago.hasPrefix("last tap: "), ago)
        XCTAssertTrue(ago.contains("2"), ago)
        XCTAssertNotEqual(ago, "last tap: never")
    }

    /// The tap window closed under the phone: the Card screen says how to open it again.
    func testACardThatMovedAwaySaysToPressTapOnTheBadge() {
        XCTAssertEqual(CardView.problem(NfcTransport.movedAway),
                       "The card moved away. Hold the phone to it until it says done. Press tap on the badge and try again.")
        XCTAssertEqual(CardView.problem("No card found. Hold the phone to the chip."),
                       "No card found. Hold the phone to the chip.")
    }

    func testTheWordsInThePocketRunLow() {
        XCTAssertNil(HelloView.pocketNote(nil))
        XCTAssertNil(HelloView.pocketNote(4))
        XCTAssertEqual(HelloView.pocketNote(3), "3 hellos left from your card. Tap the card soon.")
        XCTAssertEqual(HelloView.pocketNote(1), "1 hello left from your card. Tap the card soon.")
    }

    func testThePhoneEmptyOfWordsSaysTapTheCard() {
        XCTAssertEqual(plain(PhoneError.NeedsTap), "Tap the card first.")
    }

    /// A second badge is refused by the crate, and the pairing screen says what to do about it.
    func testASecondBadgeSaysWhatToDo() {
        XCTAssertEqual(PairCardModel.note(after: "Forget this card first."),
                       "This phone is paired to another badge. In Settings, open Card and forget that card, then pair again. The badge you just tapped already took this phone's key, so unpair it there first.")
        XCTAssertNil(PairCardModel.note(after: "No card found. Hold the phone to the chip."))
        XCTAssertNil(PairCardModel.note(after: nil))
    }
}
