import XCTest
@testable import Cahoots

final class ChannelLinkTests: XCTestCase {
    func testAChannelLinkNamesItsChannel() throws {
        let url = try XCTUnwrap(URL(string: "https://cahoots.network/app/#/ch/relay.cahoots.network/0123456789abcdef0123456789abcdef/s3cret"))
        XCTAssertEqual(AppModel.channelLinkId(url), "0123456789abcdef0123456789abcdef")
    }

    func testOtherAppLinksAreNotChannelLinks() throws {
        for s in ["https://cahoots.network/app/", "https://cahoots.network/app/#/ch/abc",
                  "https://cahoots.network/app/#/ch/relay.cahoots.network/abc",
                  "https://cahoots.network/app/#/chat/a/b/c"] {
            XCTAssertNil(AppModel.channelLinkId(try XCTUnwrap(URL(string: s))), s)
        }
    }

    func testAPolicyLineReadsAsASentence() {
        XCTAssertEqual(ChannelCopy.sentence("anyone with the link can read and post"),
                       "Anyone with the link can read and post.")
        XCTAssertEqual(ChannelRequestsView.words("read,post"), "read and post")
        XCTAssertEqual(ChannelRequestsView.words("read"), "read")
    }
}
