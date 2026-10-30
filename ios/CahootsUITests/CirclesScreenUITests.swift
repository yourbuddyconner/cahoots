import XCTest

/// The first screen renders with nothing on the phone.
final class CirclesScreenUITests: XCTestCase {
    func testTheCirclesScreenOffersToCreateAndJoin() {
        let app = XCUIApplication()
        app.launchArguments = ["-cahoots-data", "uitest-\(UUID().uuidString)"]
        app.launch()
        XCTAssertTrue(app.buttons["Create a circle"].waitForExistence(timeout: 20))
        XCTAssertTrue(app.buttons["Join a circle"].exists)
    }
}
