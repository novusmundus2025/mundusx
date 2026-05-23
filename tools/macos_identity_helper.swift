import Foundation
import CryptoKit
import Security

let service = "com.opengpu.device.identity"
let account = "default"

struct IdentityResponse: Codable {
    let public_key_hex: String
    let fingerprint: String
    let created: Bool
}

func hexString(_ data: Data) -> String {
    data.map { String(format: "%02x", $0) }.joined()
}

func fingerprint(from publicKey: Curve25519.Signing.PublicKey) -> String {
    let hex = hexString(publicKey.rawRepresentation)
    return String(hex.prefix(min(16, hex.count)))
}

func keychainQuery(returnData: Bool = false) -> [String: Any] {
    var query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service,
        kSecAttrAccount as String: account,
    ]
    if returnData {
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
    }
    return query
}

func loadSeed() throws -> Data? {
    var item: CFTypeRef?
    let status = SecItemCopyMatching(keychainQuery(returnData: true) as CFDictionary, &item)
    if status == errSecItemNotFound {
        return nil
    }
    guard status == errSecSuccess else {
        throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
    }
    return item as? Data
}

func saveSeed(_ seed: Data) throws {
    let query = keychainQuery()
    let attributes: [String: Any] = [
        kSecValueData as String: seed,
        kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
    ]

    let addStatus = SecItemAdd((query.merging(attributes, uniquingKeysWith: { current, _ in current })) as CFDictionary, nil)
    if addStatus == errSecSuccess {
        return
    }

    if addStatus == errSecDuplicateItem {
        let updateStatus = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        guard updateStatus == errSecSuccess else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(updateStatus))
        }
        return
    }

    throw NSError(domain: NSOSStatusErrorDomain, code: Int(addStatus))
}

func loadOrCreateIdentity() throws -> (privateKey: Curve25519.Signing.PrivateKey, created: Bool) {
    if let seed = try loadSeed() {
        return (try Curve25519.Signing.PrivateKey(rawRepresentation: seed), false)
    }

    let key = Curve25519.Signing.PrivateKey()
    try saveSeed(key.rawRepresentation)
    return (key, true)
}

func ensureIdentity() throws {
    let (key, created) = try loadOrCreateIdentity()
    let response = IdentityResponse(
        public_key_hex: hexString(key.publicKey.rawRepresentation),
        fingerprint: fingerprint(from: key.publicKey),
        created: created
    )
    let data = try JSONEncoder().encode(response)
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0A]))
}

func signMessage() throws {
    let stdinData = FileHandle.standardInput.readDataToEndOfFile()
    let (key, _) = try loadOrCreateIdentity()
    let signature = try key.signature(for: stdinData)
    let output = hexString(signature.rawRepresentation)
    FileHandle.standardOutput.write(Data(output.utf8))
    FileHandle.standardOutput.write(Data([0x0A]))
}

let args = CommandLine.arguments
guard args.count >= 2 else {
    fputs("usage: macos_identity_helper ensure|sign\n", stderr)
    exit(2)
}

do {
    switch args[1] {
    case "ensure":
        try ensureIdentity()
    case "sign":
        try signMessage()
    default:
        fputs("unknown command: \(args[1])\n", stderr)
        exit(2)
    }
} catch {
    fputs("\(error)\n", stderr)
    exit(1)
}
