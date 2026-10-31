import XCTest
@testable import Cahoots

/// The names the screens show for a circle's seats: a card's name first, else "Seat N".
@MainActor
final class MemberNamesTests: XCTestCase {
    private func member(_ seat: UInt8, _ name: String, device: UInt8? = nil, mine: Bool = false) -> Member {
        Member(seat: seat, name: name, faceSeed: Data(count: 32), hasCard: !name.isEmpty,
               device: device, tier: 3, mine: mine)
    }

    func testACardNameIsShownAndASeatWithoutOneKeepsItsNumber() {
        let app = AppModel()
        app.noteMembers([member(0, "Ada", mine: true), member(1, "Grace"), member(2, "")], circle: "c")
        XCTAssertEqual(app.seatName(circle: "c", seat: 1), "Grace")
        XCTAssertEqual(app.seatName(circle: "c", seat: 2), "Seat 3")
        XCTAssertTrue(app.hasCardName(circle: "c", seat: 1))
        XCTAssertFalse(app.hasCardName(circle: "c", seat: 2))
        XCTAssertEqual(app.seatName(circle: "other", seat: 1), "Seat 2")
    }

    func testTheOthersAreNamedOnceAnyHasAName() {
        let app = AppModel()
        XCTAssertNil(app.others(circle: "c", seats: [1, 2]))
        app.noteMembers([member(1, "Grace"), member(2, "")], circle: "c")
        XCTAssertEqual(app.others(circle: "c", seats: [1]), "Grace")
        XCTAssertEqual(app.others(circle: "c", seats: [1, 2]), "Grace and Seat 3")
        XCTAssertEqual(app.others(circle: "c", seats: [1, 2, 3, 4]), "Grace, Seat 3 and 2 more")
    }

    func testTheDeviceKindFollowsTheTier() {
        XCTAssertEqual(PeopleView.about(member(1, "Grace", device: 0)), "Tier 3, a hardware key store, on a phone")
        XCTAssertEqual(PeopleView.about(member(1, "Bea", device: 1)), "Tier 3, a hardware key store, on a badge")
        XCTAssertEqual(PeopleView.about(member(1, "", device: nil)), "Tier 3, a hardware key store")
    }
}
