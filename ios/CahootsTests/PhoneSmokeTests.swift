import XCTest
@testable import Cahoots

/// The Rust core through the generated bindings, with the app's own signer and HTTP.
final class PhoneSmokeTests: XCTestCase {
    private var dir: URL!
    private let signer = EnclaveSigner()

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    private func open() throws -> Phone {
        try Phone.open(dir: dir.path, signer: signer, http: UrlSessionHttp())
    }

    func testACeremonyHasCodesToShow() throws {
        let phone = try open()
        XCTAssertTrue(phone.circles().isEmpty)
        let info = phone.describe()
        XCTAssertEqual(info.fingerprint.split(separator: " ").count, 4)
        let id = try phone.startCeremony(name: "park", members: 2, padLen: 16384, throughRelay: false)
        defer {
            phone.ceremonyDiscard(id: id)
            signer.deleteKey(label: "cahoots:seat:\(id)")
        }
        let frames = try phone.ceremonyFrames(id: id)
        XCTAssertFalse(frames.isEmpty)
        XCTAssertTrue(frames[0].hasPrefix("cahoots-params:0:"), frames[0])
        let state = try phone.ceremonyState(id: id)
        XCTAssertEqual(state.phase, .commit)
        XCTAssertEqual(state.members, 2)
        let matrix = try phone.qrMatrix(text: frames[0])
        XCTAssertEqual(matrix.bits.count, Int(matrix.size * matrix.size))
    }

    func testAHostThroughTheRelayHasAnOfferCode() throws {
        let phone = try open()
        let id = try phone.startCeremony(name: "park", members: 3, padLen: 16384, throughRelay: true)
        defer { phone.ceremonyDiscard(id: id) }
        let offer = try phone.offerCode(id: id)
        XCTAssertTrue(offer.hasPrefix("cahoots:ceremony:v1:"), offer)
        let seen = try XCTUnwrap(try phone.joinScan(text: offer))
        XCTAssertEqual(seen.members, 3)
        XCTAssertTrue(seen.throughRelay)
    }

    func testTheStoreProtectsTheDirectory() throws {
        let store = try PhoneStore(root: dir)
        let values = try store.directory.resourceValues(forKeys: [.isExcludedFromBackupKey])
        XCTAssertEqual(values.isExcludedFromBackup, true)
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.directory.appendingPathComponent("circles").path))
    }

    func testOpeningTwiceKeepsThePhoneKey() throws {
        let first = try open().describe().fingerprint
        XCTAssertEqual(try open().describe().fingerprint, first)
    }
}
