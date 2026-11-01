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

    /// While a batch is open, every signature uses this one context: Face ID is asked by the
    /// first and the rest reuse it. Guarded by `batchLock`, since the crate calls from its own
    /// thread.
    private let batchLock = NSLock()
    private var batchContext: LAContext?
    private var batchDepth = 0

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
            do {
                return try publicKey(of: stored)
            } catch PhoneError.Signer {
                // A handle the enclave no longer accepts (the phone was reset, or the item is
                // damaged) can never sign. Replace it with a new key under the same name.
                deleteKey(label: label)
            }
        }
        let data: Data
        do {
            if usesEnclave {
                var error: Unmanaged<CFError>?
                guard let access = SecAccessControlCreateWithFlags(
                    nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
                    [.privateKeyUsage, .userPresence], &error
                ) else {
                    throw PhoneError.Signer(message: "Cahoots couldn’t create a key on this phone. Try again.")
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

    /// The crate opens a batch around signatures made for one request, such as a card
    /// published in every circle.
    func batchBegin() { beginBatch() }
    func batchEnd() { endBatch() }

    /// Hold one context until `endBatch`, so the signatures in between ask for Face ID once.
    func beginBatch() {
        batchLock.lock()
        defer { batchLock.unlock() }
        if batchDepth == 0 { batchContext = Self.newContext() }
        batchDepth += 1
    }

    /// Let the batch's context go. The next signature asks again.
    func endBatch() {
        batchLock.lock()
        defer { batchLock.unlock() }
        guard batchDepth > 0 else { return }
        batchDepth -= 1
        if batchDepth == 0 {
            batchContext?.invalidate()
            batchContext = nil
        }
    }

    private static func newContext() -> LAContext {
        let context = LAContext()
        context.localizedReason = "Confirm this phone’s membership in the circle"
        return context
    }

    /// The batch's context, or a fresh one outside a batch.
    private func signingContext() -> LAContext {
        batchLock.lock()
        defer { batchLock.unlock() }
        return batchContext ?? Self.newContext()
    }

    func sign(label: String, message: Data) throws -> Data {
        guard let stored = try load(label) else {
            throw PhoneError.Signer(message: "This phone no longer has the key for that seat.")
        }
        do {
            if usesEnclave {
                let context = signingContext()
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

    // MARK: - Secrets

    /// Where secrets live: apart from the keys' handles, so a key label never meets a secret's.
    static let secretService = "network.cahoots.secrets"

    private func secretQuery(_ label: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.secretService,
            kSecAttrAccount as String: label,
        ]
    }

    /// Keep a secret (the phone slot's key for a badge's chip) in a keychain item that asks for
    /// Face ID or the passcode before it is read. On the simulator, which has no enclave, it is
    /// kept under the ordinary rules. An empty secret deletes the item.
    func putSecret(label: String, secret: Data) throws {
        if secret.isEmpty {
            SecItemDelete(secretQuery(label) as CFDictionary)
            return
        }
        // An item already there takes the new secret in place, so a failure never leaves the
        // label with no key.
        let context = LAContext()
        context.localizedReason = "Save the key this phone shares with your card"
        var find = secretQuery(label)
        find[kSecUseAuthenticationContext as String] = context
        let updated = SecItemUpdate(find as CFDictionary, [kSecValueData as String: secret] as CFDictionary)
        switch updated {
        case errSecSuccess:
            return
        case errSecItemNotFound:
            break
        case errSecInteractionNotAllowed, errSecUserCanceled, errSecAuthFailed:
            throw PhoneError.Locked
        default:
            throw PhoneError.Signer(message: "Cahoots couldn’t save the card’s key on this phone (\(updated)).")
        }
        var q = secretQuery(label)
        q[kSecValueData as String] = secret
        if usesEnclave {
            var error: Unmanaged<CFError>?
            guard let access = SecAccessControlCreateWithFlags(
                nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, .userPresence, &error
            ) else {
                throw PhoneError.Signer(message: "Cahoots couldn’t protect the card’s key on this phone. Try again.")
            }
            q[kSecAttrAccessControl as String] = access
        } else {
            q[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        }
        let status = SecItemAdd(q as CFDictionary, nil)
        switch status {
        case errSecSuccess:
            return
        case errSecInteractionNotAllowed:
            throw PhoneError.Locked
        default:
            throw PhoneError.Signer(message: "Cahoots couldn’t save the card’s key on this phone (\(status)).")
        }
    }

    /// The secret kept under `label`, asking for Face ID or the passcode first, or nil if there
    /// is none.
    func getSecret(label: String) throws -> Data? {
        var q = secretQuery(label)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        let context = LAContext()
        context.localizedReason = "Use the key this phone shares with your card"
        q[kSecUseAuthenticationContext as String] = context
        var out: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &out)
        switch status {
        case errSecSuccess:
            return out as? Data
        case errSecItemNotFound:
            return nil
        case errSecInteractionNotAllowed, errSecUserCanceled, errSecAuthFailed:
            throw PhoneError.Locked
        default:
            throw PhoneError.Signer(message: "Cahoots couldn’t read the card’s key on this phone (\(status)).")
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
            throw PhoneError.Signer(message: "Cahoots couldn’t read this phone’s keychain (\(status)).")
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
            throw PhoneError.Signer(message: "Cahoots couldn’t save the key on this phone (\(status)).")
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

    /// Face ID refused, cancelled or failed, or the phone locked, becomes `Locked`, which the
    /// phone crate treats as "nothing changed, try again". Anything else (a key that does not
    /// load, no passcode set, another CryptoKit error) is a `Signer` error with its cause.
    private func mapped(_ error: Error) -> PhoneError {
        if let e = error as? PhoneError { return e }
        return Self.isRefusal(error as NSError)
            ? .Locked
            : .Signer(message: "The key could not be used: \(error.localizedDescription)")
    }

    private static let refusals: Set<Int> = [
        LAError.userCancel.rawValue, LAError.authenticationFailed.rawValue,
        LAError.systemCancel.rawValue, LAError.appCancel.rawValue,
        LAError.userFallback.rawValue, LAError.notInteractive.rawValue,
    ]

    /// The enclave reports a refused prompt as an `LAError`, or wraps one as the underlying
    /// error of what it throws.
    static func isRefusal(_ e: NSError) -> Bool {
        if e.domain == LAErrorDomain { return refusals.contains(e.code) }
        if e.domain == NSOSStatusErrorDomain {
            return e.code == Int(errSecInteractionNotAllowed) || e.code == Int(errSecUserCanceled)
                || e.code == Int(errSecAuthFailed)
        }
        return e.underlyingErrors.contains { isRefusal($0 as NSError) }
    }}
