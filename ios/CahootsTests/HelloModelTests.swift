import XCTest
@testable import Cahoots

/// The hello words screen in a card circle, with the phone and the tap stood in for: the
/// words come from a pocket that a tap on the card fills.
@MainActor
final class HelloModelTests: XCTestCase {
    private final class FakeHello: HelloSource, @unchecked Sendable {
        var number: UInt32 = 0
        /// Hello numbers held from `number` on, this one included.
        var left: UInt32
        var taps = 0
        var tapError: Error?

        init(left: UInt32) { self.left = left }

        func helloWords(circle: String, seat: UInt8) async throws -> HelloWords {
            guard left > 0 else { throw PhoneError.NeedsTap }
            return HelloWords(mine: "amber basil", theirs: "cedar delta", number: number, cardLeft: left)
        }

        func nextHello(circle: String, seat: UInt8) async throws -> HelloWords {
            guard left > 1 else { throw PhoneError.NeedsTap }
            number += 1
            left -= 1
            return try await helloWords(circle: circle, seat: seat)
        }

        func tapCard() async throws -> TapReport {
            if let tapError {
                self.tapError = nil
                throw tapError
            }
            taps += 1
            left += 8
            return TapReport(keys: 0, bumps: 0, words: 8)
        }
    }

    /// Next hello on the last pair asks for a tap. The person already confirmed, so once the
    /// tap is through the screen moves to the next hello by itself.
    func testNextHelloPastThePocketTapsThenMovesOn() async {
        let fake = FakeHello(left: 1)
        let model = HelloModel(source: fake, circle: "c", seat: 1)
        await model.load()
        XCTAssertEqual(model.words?.number, 0)
        await model.next()
        XCTAssertTrue(model.needsTap)
        XCTAssertEqual(model.error, "Tap the card first.")
        XCTAssertEqual(model.words?.number, 0)
        await model.tapCard()
        XCTAssertEqual(fake.taps, 1)
        XCTAssertFalse(model.needsTap)
        XCTAssertNil(model.error)
        XCTAssertEqual(model.words?.number, 1)
        XCTAssertEqual(model.notice, "Ready for hello 2. Check that both screens show this number.")
    }

    /// A tap that did not go through keeps the button, and the next one still moves on.
    func testAFailedTapKeepsTheButtonAndTheNextHello() async {
        let fake = FakeHello(left: 1)
        let model = HelloModel(source: fake, circle: "c", seat: 1)
        await model.load()
        await model.next()
        fake.tapError = PhoneError.NoCard
        await model.tapCard()
        XCTAssertTrue(model.needsTap)
        XCTAssertEqual(model.error, "No card found. Hold the phone still to the badge.")
        XCTAssertEqual(model.words?.number, 0)
        await model.tapCard()
        XCTAssertFalse(model.needsTap)
        XCTAssertEqual(model.words?.number, 1)
    }

    /// A tap while words remain only tops up: the number stays.
    func testATapWhileWordsRemainOnlyTopsUp() async {
        let fake = FakeHello(left: 3)
        let model = HelloModel(source: fake, circle: "c", seat: 1)
        await model.load()
        XCTAssertEqual(model.words?.cardLeft, 3)
        await model.tapCard()
        XCTAssertEqual(model.words?.number, 0)
        XCTAssertEqual(model.words?.cardLeft, 11)
        XCTAssertNil(model.notice)
    }

    /// No words at all on opening: the tap brings them, and the number stays where it was.
    func testNoWordsOnOpeningAsksForATap() async {
        let fake = FakeHello(left: 0)
        let model = HelloModel(source: fake, circle: "c", seat: 1)
        await model.load()
        XCTAssertNil(model.words)
        XCTAssertTrue(model.needsTap)
        XCTAssertEqual(model.error, "Tap the card first.")
        await model.tapCard()
        XCTAssertFalse(model.needsTap)
        XCTAssertEqual(model.words?.number, 0)
        XCTAssertNil(model.notice)
    }
}
