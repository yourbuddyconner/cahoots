import CryptoKit
import Foundation
import LocalAuthentication
import Security

/// The phone crate's `Signer`: P-256 keys that never leave this phone.
///
/// On a device each key is made in the Secure Enclave with user presence, so signing asks for
/// Face ID or the passcode. The keychain holds only the enclave's encrypted handle to the key
/// (`dataRepresentation`), readable only while the phone is unlocked and never backed up.
///
/// The simulator has no Secure Enclave. There the key is an ordinary CryptoKit key kept in the
/// keychain under the same rules, and Settings says so.
///
/// The phone crate calls this while holding its lock, from the thread that called `Phone`, so it
/// must never call back into `Phone` and never wait for the main thread.
final class EnclaveSigner: Signer {
    static let service = "network.cahoots.keys"

    /// Whether keys are made in the Secure Enclave.
    let usesEnclave: Bool

    init(enclave: Bool = EnclaveSigner.enclaveAvailable) {
        usesEnclave = enclave
    }

    /// The simulator reports the Mac's enclave as available, but cannot make keys in it.
    static var enclaveAvailable: Bool {
        #if targetEnvironment(simulator)
        return false
        #else
        return SecureEnclave.isAvailable
        #endif
    }

    func makeKey(label: String) throws -> Data {
        if let stored = try load(label) {
            return try publicKey(of: stored)
        }
        let data: Data
        do {
            if usesEnclave {
                var error: Unmanaged<CFError>?
                guard let access = SecAccessControlCreateWithFlags(
                    nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
                    [.privateKeyUsage, .userPresence], &error
                ) else {
                    throw PhoneError.Signer(message: "This phone could not set up a key.")
                }
                data = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access).dataRepresentation
            } else {
                data = P256.Signing.PrivateKey().rawRepresentation
            }
        } catch {
            throw mapped(error)
        }
        do {
            try store(label, data)
        } catch let e as PhoneError {
            // Made by someone else between the lookup and the write: theirs stands.
            if let stored = try load(label) { return try publicKey(of: stored) }
            throw e
        }
        return try publicKey(of: data)
    }

    func sign(label: String, message: Data) throws -> Data {
        guard let stored = try load(label) else {
            throw PhoneError.Signer(message: "This phone has no key for that seat.")
        }
        do {
            if usesEnclave {
                let context = LAContext()
                context.localizedReason = "Sign as your seat in the circle"
                let key = try SecureEnclave.P256.Signing.PrivateKey(
                    dataRepresentation: stored, authenticationContext: context)
                return try key.signature(for: message).rawRepresentation
            }
            let key = try P256.Signing.PrivateKey(rawRepresentation: stored)
            return try key.signature(for: message).rawRepresentation
        } catch {
            throw mapped(error)
        }
    }

    /// Forget the key named `label`. The enclave key is unusable once its handle is gone.
    func deleteKey(label: String) {
        SecItemDelete(query(label) as CFDictionary)
    }

    // MARK: - Keychain

    private func query(_ label: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.service,
            kSecAttrAccount as String: label,
        ]
    }

    private func load(_ label: String) throws -> Data? {
        var q = query(label)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &out)
        switch status {
        case errSecSuccess:
            return out as? Data
        case errSecItemNotFound:
            return nil
        case errSecInteractionNotAllowed:
            throw PhoneError.Locked
        default:
            throw PhoneError.Signer(message: "The keychain could not be read (\(status)).")
        }
    }

    private func store(_ label: String, _ data: Data) throws {
        var q = query(label)
        q[kSecValueData as String] = data
        q[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        let status = SecItemAdd(q as CFDictionary, nil)
        switch status {
        case errSecSuccess:
            return
        case errSecInteractionNotAllowed:
            throw PhoneError.Locked
        default:
            throw PhoneError.Signer(message: "The keychain could not keep the key (\(status)).")
        }
    }

    private func publicKey(of stored: Data) throws -> Data {
        do {
            if usesEnclave {
                return try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: stored)
                    .publicKey.compressedRepresentation
            }
            return try P256.Signing.PrivateKey(rawRepresentation: stored).publicKey.compressedRepresentation
        } catch {
            throw mapped(error)
        }
    }

    /// Face ID refused or cancelled, or the phone locked, becomes `Locked`, which the phone crate
    /// treats as "nothing changed, try again".
    private func mapped(_ error: Error) -> PhoneError {
        if let e = error as? PhoneError { return e }
        if error is LAError { return .Locked }
        let ns = error as NSError
        if ns.domain == LAErrorDomain { return .Locked }
        if ns.domain == NSOSStatusErrorDomain, ns.code == Int(errSecInteractionNotAllowed) { return .Locked }
        // The enclave reports a refused or cancelled Face ID prompt as a CryptoKit error.
        if usesEnclave, error is CryptoKitError { return .Locked }
        return .Signer(message: "The key could not be used: \(error.localizedDescription)")
    }
}
