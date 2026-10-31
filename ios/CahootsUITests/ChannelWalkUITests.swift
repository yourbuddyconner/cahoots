import XCTest

/// A channel between two simulators, driven through the app's screens and photographed at
/// each step: the owner starts one from a circle, shares the link, the reader opens it, asks
/// to post, is let in, posts, and the owner reads it. Each side runs on its own simulator, on
/// the circles the relay walk left there, and they hand each other the link and the reader's
/// name through files in `CAHOOTS_WALK_DIR`. Skipped unless `CAHOOTS_WALK_DIR` and the side's
/// data folder (`CAHOOTS_DATA`) are set; `scripts/ios-channel-walk.sh` has the commands.
final class ChannelWalkUITests: XCTestCase {
    private var dir: URL!
    private var data = ""
    private let app = XCUIApplication()

    override func setUpWithError() throws {
        let env = ProcessInfo.processInfo.environment
        guard let d = env["CAHOOTS_WALK_DIR"], let data = env["CAHOOTS_DATA"] else {
            throw XCTSkip("The two-simulator channel walk runs only when asked. See scripts/ios-channel-walk.sh.")
        }
        dir = URL(fileURLWithPath: d)
        self.data = data
        continueAfterFailure = false
        // Whatever was on screen when a step failed.
        addTeardownBlock { [self] in shot("\(name.contains("Owner") ? "owner" : "reader")-end") }
    }

    func testOwner() throws {
        launch()
        shot("owner-01-circles")
        let start = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Start a channel'")).firstMatch
        XCTAssertTrue(start.waitForExistence(timeout: 20))
        start.tap()
        let name = element("channel name")
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        shot("owner-02-start-sheet")
        name.tap()
        name.typeText("Garden news\n")
        let group = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Group chat'")).firstMatch
        XCTAssertTrue(group.waitForExistence(timeout: 5))
        group.tap()
        shot("owner-03-start-filled")
        app.buttons["Start"].tap()

        let link = element("channel link")
        XCTAssertTrue(link.waitForExistence(timeout: 60))
        shot("owner-04-share")
        try write("link.txt", link.label)
        app.buttons["Done"].tap()
        shot("owner-05-channel-empty")

        let field = element("message")
        XCTAssertTrue(field.waitForExistence(timeout: 20))
        field.tap()
        field.typeText("First post from the garden")
        app.buttons["send"].tap()
        XCTAssertTrue(app.staticTexts["First post from the garden"].waitForExistence(timeout: 60))
        shot("owner-06-channel-posted")

        app.buttons["Channel settings"].tap()
        shot("owner-07-menu")
        app.buttons["Requests"].tap()
        let reader = try wait(for: "reader.txt")
        XCTAssertTrue(app.staticTexts[reader].waitForExistence(timeout: 180))
        shot("owner-08-requests")
        app.buttons["Allow to post"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["\(reader) can read and post."].waitForExistence(timeout: 60))
        shot("owner-09-allowed")
        app.navigationBars.buttons.element(boundBy: 0).tap()

        XCTAssertTrue(app.staticTexts["Hello from the reader"].waitForExistence(timeout: 180))
        shot("owner-10-channel-reply")
        app.buttons["Channel settings"].tap()
        app.buttons["Members"].tap()
        XCTAssertTrue(app.staticTexts[reader].waitForExistence(timeout: 20))
        shot("owner-11-members")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        app.navigationBars.buttons.element(boundBy: 0).tap()
        shot("owner-12-circles-with-channel")
    }

    func testReader() throws {
        let link = try wait(for: "link.txt")
        launch(link: link)
        XCTAssertTrue(app.buttons["Open"].waitForExistence(timeout: 60))
        shot("reader-01-join")
        app.buttons["Open"].tap()
        XCTAssertTrue(app.staticTexts["First post from the garden"].waitForExistence(timeout: 120))
        shot("reader-02-channel-reading")
        app.buttons["Ask to post"].firstMatch.tap()
        let name = app.textFields.firstMatch
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        shot("reader-03-ask")
        name.tap()
        name.typeText("Bea")
        app.buttons["Ask to post"].firstMatch.tap()
        try write("reader.txt", "Bea")
        XCTAssertTrue(app.staticTexts["Waiting for the owner to let you post."].waitForExistence(timeout: 30))
        shot("reader-04-waiting")

        let field = element("message")
        XCTAssertTrue(field.waitForExistence(timeout: 240))
        shot("reader-05-allowed")
        field.tap()
        field.typeText("Hello from the reader")
        app.buttons["send"].tap()
        XCTAssertTrue(app.staticTexts["Hello from the reader"].waitForExistence(timeout: 60))
        shot("reader-06-posted")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        shot("reader-07-circles-with-channel")
    }

    // MARK: - Steps

    private func launch(link: String? = nil) {
        var args = ["-cahoots-data", data]
        if let link { args += ["-cahoots-link", link] }
        app.launchArguments = args
        app.launch()
        XCTAssertTrue(app.navigationBars.firstMatch.waitForExistence(timeout: 30))
    }

    private func element(_ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
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
        Thread.sleep(forTimeInterval: 0.6)
        let png = XCUIScreen.main.screenshot().pngRepresentation
        try? png.write(to: dir.appendingPathComponent("\(name).png"))
    }
}
