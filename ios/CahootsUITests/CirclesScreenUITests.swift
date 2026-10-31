import XCTest

/// The first screen renders with nothing on the phone.
final class CirclesScreenUITests: XCTestCase {
    func testTheCirclesScreenOffersToCreateAndJoin() {
        let app = XCUIApplication()
        app.launchArguments = ["-cahoots-data", "uitest-\(UUID().uuidString)", "-cahoots.welcomeSeen", "YES"]
        app.launch()
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 20))
        XCTAssertTrue(app.buttons["Join a circle"].exists)
    }
}

/// First-use guidance can be completed, skipped, and reopened without returning at every launch.
final class WelcomeUITests: XCTestCase {
    func testWelcomeCompletionAndReopening() {
        let app = XCUIApplication()
        let data = "welcome-\(UUID().uuidString)"
        app.launchArguments = ["-cahoots-data", data, "-cahoots.welcomeSeen", "NO"]
        app.launch()
        for index in 0..<4 {
            XCTAssertTrue(app.descendants(matching: .any)["welcome-page-\(index)"].waitForExistence(timeout: 20))
            if let output = ProcessInfo.processInfo.environment["CAHOOTS_COPY_SHOTS"] {
                try? app.screenshot().pngRepresentation.write(to: URL(fileURLWithPath: output).appendingPathComponent("welcome-\(index).png"))
            }
            if index == 1 { app.swipeUp() }
            app.buttons["welcome-next"].tap()
        }
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 10))
        app.terminate()
        app.launchArguments = ["-cahoots-data", data]
        app.launch()
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 20))
        XCTAssertFalse(app.buttons["welcome-next"].exists)
        app.buttons["Settings"].tap()
        app.buttons["How Cahoots works"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["welcome-page-0"].waitForExistence(timeout: 10))
        app.buttons["welcome-next"].tap()
        app.buttons["welcome-back"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["welcome-page-0"].exists)
        app.buttons["welcome-close"].tap()
        XCTAssertTrue(app.buttons["How Cahoots works"].waitForExistence(timeout: 10))
    }

    func testWelcomeCanBeSkipped() {
        let app = XCUIApplication()
        let data = "welcome-skip-\(UUID().uuidString)"
        app.launchArguments = ["-cahoots-data", data, "-cahoots.welcomeSeen", "NO"]
        app.launch()
        XCTAssertTrue(app.buttons["welcome-close"].waitForExistence(timeout: 20))
        app.buttons["welcome-close"].tap()
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 10))
        app.terminate()
        app.launchArguments = ["-cahoots-data", data]
        app.launch()
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 20))
        XCTAssertFalse(app.buttons["welcome-next"].exists)
    }
}
