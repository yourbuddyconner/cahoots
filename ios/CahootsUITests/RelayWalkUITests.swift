import CoreImage
import XCTest

/// A ceremony through the relay between two simulators, then a chat, driven through the app's
/// screens. Each side runs on its own simulator at the same time, and they hand each other the
/// offer and join codes through files in `CAHOOTS_WALK_DIR`: each side reads the other's code
/// off its own screen (a screenshot of the code, decoded) and the other side pastes it, since a
/// simulator has no camera. Skipped unless `CAHOOTS_WALK_DIR` and `CAHOOTS_STATION` are set;
/// `ios/README.md` has the commands. With `CAHOOTS_CARDS` set, the host makes that many seats
/// for cards in a pile behind the bridge, after its own, and pastes each card's join code from
/// `card-<seat>.txt`, which `scripts/bridge-bench.sh` writes from the bridge's output.
final class RelayWalkUITests: XCTestCase {
    private var dir: URL!
    private var station = ""
    private var cards = 0
    private let app = XCUIApplication()

    override func setUpWithError() throws {
        let env = ProcessInfo.processInfo.environment
        guard let d = env["CAHOOTS_WALK_DIR"], let s = env["CAHOOTS_STATION"] else {
            throw XCTSkip("The two-simulator walk runs only when asked. See ios/README.md.")
        }
        dir = URL(fileURLWithPath: d)
        station = s
        cards = Int(env["CAHOOTS_CARDS"] ?? "") ?? 0
        continueAfterFailure = false
    }

    func testHost() throws {
        launch("walk-host")
        // The relay for new circles: the local station.
        app.buttons["Settings"].tap()
        let relay = element("default-relay")
        XCTAssertTrue(relay.waitForExistence(timeout: 10))
        // Tap at the end of the address so the deletes clear all of it.
        relay.tap()
        relay.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
        relay.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 60) + station)
        app.buttons["save-relay"].tap()
        XCTAssertTrue(app.staticTexts["Saved. New circles use this relay."].waitForExistence(timeout: 10))
        shot("host-1-settings")
        app.navigationBars.buttons.element(boundBy: 0).tap()

        app.buttons["Create a circle"].tap()
        let name = element("circle-name")
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        name.tap()
        name.typeText("park")
        if cards > 0 {
            // The host, the cards, and one joiner on the other simulator.
            name.typeText("\n")
            let people = element("people")
            for _ in 0..<cards { plus(people) }
            XCTAssertEqual(people.value as? String, "\(cards + 2)")
            let pile = element("cards")
            for _ in 0..<cards { plus(pile) }
            XCTAssertEqual(pile.value as? String, "\(cards)")
        }
        shot("host-2-new-circle")
        app.buttons["start-ceremony"].tap()

        let offer = try readCode()
        XCTAssertTrue(offer.hasPrefix(cards > 0 ? "cahoots:ceremony:v2:" : "cahoots:ceremony:v1:"), offer)
        shot("host-3-offer")
        if cards > 0 {
            // Each card's seat row says it comes through the bridge.
            for seat in 2...(cards + 1) {
                let row = app.descendants(matching: .any).matching(NSPredicate(
                    format: "label BEGINSWITH %@ AND label CONTAINS 'card, through the bridge'", "Seat \(seat)"))
                XCTAssertTrue(row.firstMatch.exists, "seat \(seat) is not shown as a card")
            }
            app.swipeUp()
            shot("host-3-seats")
        }
        try write("offer.txt", offer)

        if cards > 0 {
            for seat in 2...(cards + 1) {
                let code = try wait(for: "card-\(seat).txt")
                app.buttons["scan-joiner"].tap()
                paste(code)
                XCTAssertTrue(app.staticTexts["Scanned seat \(seat)"].waitForExistence(timeout: 20))
                shot("host-4-scanned-card-\(seat)")
            }
        }
        let join = try wait(for: "join.txt")
        app.buttons["scan-joiner"].tap()
        paste(join)
        XCTAssertTrue(app.staticTexts["Scanned seat \(cards + 2)"].waitForExistence(timeout: 20))
        shot("host-4-scanned")
        let proceed = app.buttons["continue"]
        XCTAssertTrue(proceed.waitForExistence(timeout: 60))
        shot("host-5-everyone-scanned")
        proceed.tap()

        try compareWords(side: "host")

        let field = element("message")
        XCTAssertTrue(field.waitForExistence(timeout: 20))
        field.tap()
        field.typeText("hello from the host")
        app.buttons["send"].tap()
        XCTAssertTrue(app.staticTexts["hello from the joiner"].waitForExistence(timeout: 120))
        shot("host-7-chat")
        tour(side: "host")
    }

    func testJoiner() throws {
        launch("walk-joiner")
        shot("joiner-0-empty")
        let offer = try wait(for: "offer.txt")
        app.buttons["Join a circle"].tap()
        paste(offer)
        let contribute = app.buttons["make-contribution"]
        XCTAssertTrue(contribute.waitForExistence(timeout: 20))
        shot("joiner-1-seat")
        contribute.tap()

        let join = try readCode()
        XCTAssertTrue(join.hasPrefix("cahoots:joined:v1:"), join)
        shot("joiner-2-join-code")
        try write("join.txt", join)

        try compareWords(side: "joiner")

        XCTAssertTrue(app.staticTexts["hello from the host"].waitForExistence(timeout: 120))
        let field = element("message")
        field.tap()
        field.typeText("hello from the joiner")
        app.buttons["send"].tap()
        XCTAssertTrue(app.staticTexts["hello from the joiner"].waitForExistence(timeout: 20))
        shot("joiner-4-chat")
        tour(side: "joiner")
    }

    /// After the chat: the roster, hello words, the list of circles and settings, for the record.
    private func tour(side: String) {
        app.buttons["People in park"].tap()
        // The other phone's seat, whose message is in the chat: after the cards for the host.
        let seat = side == "host" ? cards + 1 : 0
        let other = element("hello-seat-\(seat)")
        XCTAssertTrue(other.waitForExistence(timeout: 10))
        shot("\(side)-8-people")
        element("edit-seat-\(seat)").tap()
        let name = element("seat-name")
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        name.tap()
        name.typeText("Friend")
        element("save-seat-name").tap()
        XCTAssertTrue(app.staticTexts["Friend"].waitForExistence(timeout: 10))
        shot("\(side)-8-named-people")
        other.tap()
        XCTAssertTrue(app.buttons["Next hello"].waitForExistence(timeout: 10))
        XCTAssertEqual(element("hello-number").label, "Hello 1")
        shot("\(side)-9-hello")
        app.buttons["Next hello"].tap()
        app.buttons["Cancel"].tap()
        XCTAssertEqual(element("hello-number").label, "Hello 1")
        app.buttons["Next hello"].tap()
        app.buttons["Move to next hello"].tap()
        XCTAssertTrue(app.staticTexts["Hello 2"].waitForExistence(timeout: 10))
        shot("\(side)-9-next-hello")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.staticTexts["Friend"].waitForExistence(timeout: 10))
        // Names survive reopening the phone and remain scoped to this circle.
        app.terminate()
        app.launch()
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 20))
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'park.'")).firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Friend"].waitForExistence(timeout: 10))
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 10))
        shot("\(side)-10-circles")
        app.buttons["Settings"].tap()
        XCTAssertTrue(element("default-relay").waitForExistence(timeout: 10))
        shot("\(side)-11-settings")
    }

    // MARK: - Steps

    private func launch(_ data: String) {
        app.launchArguments = ["-cahoots-data", "\(data)-\(UUID().uuidString)", "-cahoots.welcomeSeen", "YES"]
        app.launch()
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 30))
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// The plus button of a stepper: the last 44 points of its row. The row is one element, so
    /// the button has no element of its own.
    private func plus(_ stepper: XCUIElement) {
        XCTAssertTrue(stepper.waitForExistence(timeout: 10))
        stepper.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5))
            .withOffset(CGVector(dx: stepper.frame.width - 22, dy: 0)).tap()
    }

    private func paste(_ code: String) {
        let field = element("paste-code")
        XCTAssertTrue(field.waitForExistence(timeout: 20))
        // A sheet may still be sliding in. Tap until the keyboard is up.
        for _ in 0..<5 where !app.keyboards.firstMatch.exists {
            field.tap()
            _ = app.keyboards.firstMatch.waitForExistence(timeout: 2)
        }
        field.typeText(code)
        app.buttons["use-code"].tap()
    }

    /// Both sides see six words. Each writes its own, and the shell compares them.
    private func compareWords(side: String) throws {
        let match = app.buttons["They match"]
        XCTAssertTrue(match.waitForExistence(timeout: 180))
        let words = element("words").label
        shot("\(side)-words")
        try write("\(side)-words.txt", words)
        match.tap()
    }

    /// Decode the code on screen from a screenshot of it.
    private func readCode() throws -> String {
        let code = element("code")
        XCTAssertTrue(code.waitForExistence(timeout: 60))
        let detector = CIDetector(ofType: CIDetectorTypeQRCode, context: nil,
                                  options: [CIDetectorAccuracy: CIDetectorAccuracyHigh])
        for _ in 0..<20 {
            if let image = CIImage(image: code.screenshot().image),
               let found = detector?.features(in: image).compactMap({ ($0 as? CIQRCodeFeature)?.messageString }).first {
                return found
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTFail("could not read the code on screen")
        return ""
    }

    private func write(_ name: String, _ text: String) throws {
        try text.write(to: dir.appendingPathComponent(name), atomically: true, encoding: .utf8)
    }

    private func wait(for name: String) throws -> String {
        let url = dir.appendingPathComponent(name)
        for _ in 0..<600 {
            if let s = try? String(contentsOf: url, encoding: .utf8), !s.isEmpty { return s }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTFail("\(name) never came")
        return ""
    }

    private func shot(_ name: String) {
        let png = XCUIScreen.main.screenshot().pngRepresentation
        try? png.write(to: dir.appendingPathComponent("\(name).png"))
    }
}
