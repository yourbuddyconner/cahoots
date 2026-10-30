import CryptoKit
import XCTest
@testable import Cahoots

/// The signer the app hands the phone crate. On the simulator it takes the keychain path,
/// on a device the Secure Enclave; both must answer the same way.
final class EnclaveSignerTests: XCTestCase {
    private let signer = EnclaveSigner()
    private var labels: [String] = []

    private func label() -> String {
        let l = "cahoots:test:\(UUID().uuidString)"
        labels.append(l)
        return l
    }

    override func tearDown() {
        for l in labels { signer.deleteKey(label: l) }
        super.tearDown()
    }

    func testMakeKeySignAndVerify() throws {
        let l = label()
        let publicKey = try signer.makeKey(label: l)
        XCTAssertEqual(publicKey.count, 33, "SEC1 compressed")
        let message = Data("the manifest".utf8)
        let signature = try signer.sign(label: l, message: message)
        XCTAssertEqual(signature.count, 64, "raw r || s")
        let key = try P256.Signing.PublicKey(compressedRepresentation: publicKey)
        let sig = try P256.Signing.ECDSASignature(rawRepresentation: signature)
        XCTAssertTrue(key.isValidSignature(sig, for: message))
        XCTAssertFalse(key.isValidSignature(sig, for: Data("another".utf8)))
    }

    func testMakeKeyTwiceReturnsTheSameKey() throws {
        let l = label()
        let first = try signer.makeKey(label: l)
        let second = try signer.makeKey(label: l)
        XCTAssertEqual(first, second)
        XCTAssertNotEqual(try signer.makeKey(label: label()), first)
    }

    func testSigningWithNoKeyIsAnError() {
        XCTAssertThrowsError(try signer.sign(label: label(), message: Data([1]))) { error in
            guard case PhoneError.Signer = error else {
                return XCTFail("expected PhoneError.Signer, got \(error)")
            }
        }
    }

    func testADeletedKeyIsGone() throws {
        let l = label()
        let first = try signer.makeKey(label: l)
        signer.deleteKey(label: l)
        XCTAssertNotEqual(try signer.makeKey(label: l), first)
    }
}
