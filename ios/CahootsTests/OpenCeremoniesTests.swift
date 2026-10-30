import XCTest
@testable import Cahoots

final class OpenCeremoniesTests: XCTestCase {
    private var defaults: UserDefaults!
    private var suite = ""

    override func setUp() {
        suite = "cahoots.tests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
    }

    func testARecordedCeremonyIsClearedWhenKeptOrDiscarded() {
        let open = OpenCeremonies(defaults: defaults)
        open.record("aaaa")
        open.record("bbbb")
        open.record("aaaa")
        XCTAssertEqual(open.ids, ["aaaa", "bbbb"])
        open.clear("aaaa")
        XCTAssertEqual(open.ids, ["bbbb"])
        open.clear("bbbb")
        XCTAssertEqual(open.ids, [])
        XCTAssertNil(defaults.object(forKey: OpenCeremonies.key))
        XCTAssertEqual(open.takeLost(), [], "nothing open, nothing lost")
    }

    func testACeremonyStillRecordedAtLaunchIsLostOnce() {
        OpenCeremonies(defaults: defaults).record("cafe")
        // The app closed. At the next launch:
        let open = OpenCeremonies(defaults: defaults)
        XCTAssertEqual(open.takeLost(), ["cahoots:seat:cafe"])
        XCTAssertEqual(open.takeLost(), [], "told once")
    }

    func testTheLostCeremonysKeyIsDeleted() throws {
        let signer = EnclaveSigner()
        let label = OpenCeremonies.keyLabel("dead\(UUID().uuidString.prefix(8))")
        let first = try signer.makeKey(label: label)
        signer.deleteKey(label: label)
        XCTAssertThrowsError(try signer.sign(label: label, message: Data([1])))
        XCTAssertNotEqual(try signer.makeKey(label: label), first)
        signer.deleteKey(label: label)
    }
}
