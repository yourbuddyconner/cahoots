import XCTest
@testable import Cahoots

final class ChannelLinkTests: XCTestCase {
    func testAChannelLinkNamesItsChannel() throws {
        let url = try XCTUnwrap(URL(string: "https://cahoots.network/app/#/ch/relay.cahoots.network/0123456789abcdef0123456789abcdef/s3cret"))
        XCTAssertEqual(AppModel.channelLinkId(url), "0123456789abcdef0123456789abcdef")
    }

    func testOtherAppLinksAreNotChannelLinks() throws {
        for s in ["https://cahoots.network/app/", "https://cahoots.network/app/#/ch/abc",
                  "https://cahoots.network/app/#/ch/a/b/c/d",
                  "https://cahoots.network/app/#/chat/a/b/c"] {
            XCTAssertNil(AppModel.channelLinkId(try XCTUnwrap(URL(string: s))), s)
        }
    }

    private let id = "0123456789abcdef0123456789abcdef"
    private func link(_ s: String) -> URL { URL(string: s)! }
    private var channelLink: URL { link("https://cahoots.network/app/#/ch/relay.cahoots.network/\(id)/s3cret") }
    private func summary(_ role: ChannelRole) -> ChannelSummary {
        ChannelSummary(id: id, name: "Garden news", role: role, policyLine: "anyone with the link can read",
                       relay: "relay.cahoots.network", dead: false, canPost: role == .owner, waiting: false,
                       unread: 0, mayAskToPost: false, closed: false, circle: nil)
    }

    func testAnOwnersLinkOpensTheChannel() {
        XCTAssertEqual(AppModel.linkAction(channelLink, ready: true, path: [], channels: [summary(.owner)]),
                       .open(.channel(id)))
    }

    func testANewLinkOpensTheJoinScreenOnTopOfThePath() {
        XCTAssertEqual(AppModel.linkAction(channelLink, ready: true, path: [.chat("c")], channels: []),
                       .open(.joinChannel(channelLink.absoluteString)))
    }

    func testALinkToAChannelAlreadyReadSkipsTheJoinScreen() {
        XCTAssertEqual(AppModel.linkAction(channelLink, ready: true, path: [], channels: [summary(.member)]),
                       .rejoin(id: id, link: channelLink.absoluteString))
    }

    func testOtherAppLinksGoToSafariAndOtherSitesAreIgnored() {
        XCTAssertEqual(AppModel.linkAction(link("https://cahoots.network/app/#/c/abc"), ready: true, path: [], channels: []), .safari)
        XCTAssertEqual(AppModel.linkAction(link("https://cahoots.network/app/"), ready: true, path: [], channels: []), .safari)
        XCTAssertEqual(AppModel.linkAction(link("https://example.com/app/#/ch/a/b/c"), ready: true, path: [], channels: []), .ignore)
        XCTAssertEqual(AppModel.linkAction(link("https://cahoots.network/blog/"), ready: true, path: [], channels: []), .ignore)
    }

    func testALinkBeforeThePhoneOpensIsQueued() {
        XCTAssertEqual(AppModel.linkAction(channelLink, ready: false, path: [], channels: []), .queue)
    }

    func testALinkDuringACeremonyIsHeld() {
        XCTAssertEqual(AppModel.linkAction(channelLink, ready: true, path: [.newCircle, .ceremony("x")], channels: []), .hold)
    }

    func testALinkWithoutItsSecretIsAChannelLink() {
        let closed = link("https://cahoots.network/app/#/ch/relay.cahoots.network/\(id)")
        XCTAssertEqual(AppModel.channelLinkId(closed), id)
        XCTAssertEqual(AppModel.linkAction(closed, ready: true, path: [], channels: []), .open(.joinChannel(closed.absoluteString)))
    }

    func testTheLinkPassedOnIsCanonical() {
        let odd = link("https://cahoots.network/app?x=1#/ch/relay.cahoots.network/\(id)/s3cret")
        XCTAssertEqual(AppModel.canonicalLink(odd), channelLink.absoluteString)
        XCTAssertEqual(AppModel.linkAction(odd, ready: true, path: [], channels: []),
                       .open(.joinChannel(channelLink.absoluteString)))
    }

    func testAPolicyLineReadsAsASentence() {
        XCTAssertEqual(ChannelCopy.sentence("anyone with the link can read and post"),
                       "Anyone with the link can read and post.")
        XCTAssertEqual(ChannelRequestsView.words("read,post"), "read and post")
        XCTAssertEqual(ChannelRequestsView.words("read"), "read")
    }
}
