import XCTest
@testable import Cahoots

/// Against a real `cahoots-station`. iOS tests cannot start a process, so
/// `scripts/ios-build.sh test` starts one and passes its address as `CAHOOTS_STATION`.
final class UrlSessionHttpTests: XCTestCase {
    private func station() throws -> String {
        guard let url = ProcessInfo.processInfo.environment["CAHOOTS_STATION"], !url.isEmpty else {
            throw XCTSkip("No station. Run scripts/ios-build.sh test, which starts one.")
        }
        return url
    }

    func testPostThenGet() throws {
        let base = try station()
        let http = UrlSessionHttp()
        let id = (0..<16).map { _ in UInt8.random(in: 0...255) }
        let circle = id.map { String(format: "%02x", $0) }.joined()
        let url = "\(base)/v1/circles/\(circle)/messages"
        // A ceremony message: version 3, the channel id, the length, the payload.
        let payload = Data((0..<40).map { UInt8($0) })
        var body = Data([3]) + Data(id)
        withUnsafeBytes(of: UInt32(payload.count).littleEndian) { body.append(contentsOf: $0) }
        body.append(payload)
        let answer = try http.post(url: url, body: body)
        XCTAssertFalse(answer.isEmpty)
        let got = try http.get(url: "\(url)?wait=0")
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: got) as? [String: Any])
        let entries = try XCTUnwrap(json["entries"] as? [[String: Any]])
        XCTAssertEqual(entries.count, 1)
        let message = try XCTUnwrap(entries[0]["message"] as? String)
        XCTAssertEqual(Data(base64URL: message), body)
    }

    func testANon2xxAnswerIsANetworkErrorWithTheStatus() throws {
        let base = try station()
        XCTAssertThrowsError(try UrlSessionHttp().get(url: "\(base)/v1/circles/not-hex/messages")) { error in
            guard case let PhoneError.Network(message) = error else {
                return XCTFail("expected PhoneError.Network, got \(error)")
            }
            XCTAssertTrue(message.hasPrefix("The relay "), message)
            XCTAssertNotEqual(message, UrlSessionHttp.unreachable)
        }
    }

    func testARefusalSaysTheRelaysDetailOrItsStatus() {
        let body = Data(#"{"detail":"unknown message version","error":"malformed"}"#.utf8)
        XCTAssertEqual(UrlSessionHttp.refusal(status: 400, body: body, method: "POST"),
                       "The relay refused the message: unknown message version")
        XCTAssertEqual(UrlSessionHttp.refusal(status: 400, body: body, method: "GET"),
                       "The relay refused the request: unknown message version")
        XCTAssertEqual(UrlSessionHttp.refusal(status: 500, body: Data("oops".utf8), method: "POST"),
                       "The relay answered 500.")
        XCTAssertEqual(UrlSessionHttp.refusal(status: 502, body: nil, method: "GET"),
                       "The relay answered 502.")
    }

    func testRequestReturnsARefusalWithItsStatus() throws {
        let base = try station()
        let answer = try UrlSessionHttp().request(method: "GET", url: "\(base)/v1/circles/not-hex/messages",
                                                  headers: [HttpHeader(name: "Cahoots-Kid", value: "00")], body: nil)
        XCTAssertGreaterThanOrEqual(answer.status, 400)
        XCTAssertLessThan(answer.status, 500)
    }

    func testRequestWithNoRelayIsANetworkError() {
        XCTAssertThrowsError(try UrlSessionHttp().request(method: "GET", url: "http://127.0.0.1:9/v1/head",
                                                          headers: [], body: nil)) { error in
            guard case let PhoneError.Network(message) = error else {
                return XCTFail("expected PhoneError.Network, got \(error)")
            }
            XCTAssertEqual(message, UrlSessionHttp.unreachable)
        }
    }

    func testNoRelayIsANetworkError() {
        XCTAssertThrowsError(try UrlSessionHttp().get(url: "http://127.0.0.1:9/v1/head")) { error in
            guard case let PhoneError.Network(message) = error else {
                return XCTFail("expected PhoneError.Network, got \(error)")
            }
            XCTAssertEqual(message, UrlSessionHttp.unreachable)
        }
    }
}

private extension Data {
    /// The station sends messages as base64, URL-safe or not, with or without padding.
    init?(base64URL s: String) {
        var t = s.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while t.count % 4 != 0 { t += "=" }
        self.init(base64Encoded: t)
    }
}
