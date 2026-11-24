import SwiftUI
import Foundation
import Security
import CryptoKit

// Test-only host. Credentials are kept in the app's Keychain, never in its documents.
final class CheckCredentials: SaveRelayCredentials {
    private func query(_ account: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "save-relay-relay-check",
         kSecAttrAccount as String: account]
    }
    func secret(for account: String) -> Data? {
        var request = query(account)
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(request as CFDictionary, &result) == errSecSuccess else { return nil }
        return result as? Data
    }
    func setSecret(_ secret: String, for account: String) -> Bool {
        let item = query(account)
        let data = Data(secret.utf8)
        let updated = SecItemUpdate(item as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if updated == errSecSuccess { return true }
        guard updated == errSecItemNotFound else { return false }
        var newItem = item
        newItem[kSecValueData as String] = data
        newItem[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(newItem as CFDictionary, nil) == errSecSuccess
    }
    func deleteSecret(for account: String) -> Bool {
        let status = SecItemDelete(query(account) as CFDictionary)
        return status == errSecSuccess || status == errSecItemNotFound
    }
}

// The callback is synchronous. URLSession runs on a separate delegate queue; never call the core
// on the UI thread. A bounded response is stopped before it can grow past the requested limit.
final class CheckNetwork: SaveRelayNetwork {
    func respond(to request: SaveRelayRequest) -> SaveRelayResponse? {
        CheckTransfer().run(request)
    }
}

// A fresh delegate per request isolates a late timeout callback from later requests.
final class CheckTransfer: NSObject, URLSessionDataDelegate {
    private let lock = NSLock()
    private var bytes = Data()
    private var response: HTTPURLResponse?
    private var failed = false
    private var limit = 0
    private var done = DispatchSemaphore(value: 0)

    func run(_ request: SaveRelayRequest) -> SaveRelayResponse? {
        guard let url = URL(string: request.url), url.scheme == "https" else { return nil }
        bytes = Data(); response = nil; failed = false
        limit = request.maxResponseBytes > 0 ? request.maxResponseBytes : 256 * 1024 * 1024
        done = DispatchSemaphore(value: 0)
        var outbound = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 60)
        outbound.httpMethod = request.method
        for line in request.headers.split(separator: "\n") {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let name = String(line[..<colon]).trimmingCharacters(in: .whitespaces)
            let value = String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
            outbound.setValue(value, forHTTPHeaderField: name)
        }
        outbound.setValue("identity", forHTTPHeaderField: "Accept-Encoding")
        if !request.body.isEmpty { outbound.httpBody = request.body }
        let session = URLSession(configuration: .ephemeral, delegate: self, delegateQueue: nil)
        let task = session.dataTask(with: outbound)
        task.resume()
        if done.wait(timeout: .now() + 120) == .timedOut { task.cancel(); session.invalidateAndCancel(); return nil }
        session.finishTasksAndInvalidate()
        lock.lock(); defer { lock.unlock() }
        guard !failed, let response = response else { return nil }
        let headers = response.allHeaderFields.compactMap { key, value -> String? in
            guard let name = key as? String else { return nil }
            return "\(name): \(value)"
        }.joined(separator: "\n")
        return SaveRelayResponse(status: UInt16(response.statusCode), headers: headers, body: bytes)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        lock.lock(); self.response = response as? HTTPURLResponse; lock.unlock()
        completionHandler(.allow)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock(); defer { lock.unlock() }
        if data.count > limit - bytes.count { failed = true; dataTask.cancel(); return }
        bytes.append(data)
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.lock(); if error != nil { failed = true }; lock.unlock(); done.signal()
    }
}

final class CheckModel: ObservableObject {
    @Published var server = ""
    @Published var user = ""
    @Published var password = ""
    @Published var content = "iPhone test save"
    @Published var output = "Ready"
    @Published var busy = false
    private let queue = DispatchQueue(label: "relay-check")
    private var relay: SaveRelay?
    private let manifest = #"{"format":1,"id":"relay-check","name":"Relay Check","saves":{},"saves_from":"launcher","include":["**/*"]}"#
    private let root: URL
    private let saves: URL

    init() {
        let directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("relay-check", isDirectory: true)
        root = directory
        saves = directory.appendingPathComponent("saves", isDirectory: true)
    }
    func perform(_ operation: String) {
        guard !busy else { return }
        let url = server, account = user, secret = password, text = content
        password = ""; busy = true
        queue.async {
            let message: String
            do {
                try FileManager.default.createDirectory(at: self.saves, withIntermediateDirectories: true)
                if self.relay == nil {
                    self.relay = SaveRelay(home: self.root.path, data: self.root.path,
                                           config: self.root.path, credentials: CheckCredentials(),
                                           network: CheckNetwork())
                }
                guard let relay = self.relay else { throw CheckError.initialization }
                let file = self.saves.appendingPathComponent("check.txt")
                switch operation {
                case "Setup":
                    message = relay.setup(server: url, user: account, password: secret,
                                          device: "iphone-check", root: "save-relay-ios-check")
                case "Push":
                    try Data(text.utf8).write(to: file, options: .atomic)
                    let envelope = relay.push(manifestJson: self.manifest, savesDir: self.saves.path)
                    let hash = SHA256.hash(data: Data(text.utf8)).map { String(format: "%02x", $0) }.joined()
                    message = envelope + "\ncheck.txt SHA-256: " + hash
                default:
                    let envelope = relay.pull(manifestJson: self.manifest, savesDir: self.saves.path)
                    if let data = try? Data(contentsOf: file) {
                        let hash = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
                        message = envelope + "\ncheck.txt SHA-256: " + hash
                    } else { message = envelope }
                }
            } catch { message = "Test app error: \(error.localizedDescription)" }
            DispatchQueue.main.async { self.output = message; self.busy = false }
        }
    }
    enum CheckError: Error { case initialization }
}

@main
struct RelayCheckApp: App {
    @StateObject private var model = CheckModel()
    var body: some Scene {
        WindowGroup {
            Form {
                Section("Nextcloud test account") {
                    TextField("HTTPS server", text: $model.server).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("User", text: $model.user).textInputAutocapitalization(.never).autocorrectionDisabled()
                    SecureField("App password", text: $model.password)
                    Button("Setup") { model.perform("Setup") }
                }
                Section("relay-check") {
                    TextField("Contents for next push", text: $model.content)
                    Button("Pull") { model.perform("Pull") }
                    Button("Write and push") { model.perform("Push") }
                }
                Section("Result") { Text(model.output).textSelection(.enabled) }
            }.disabled(model.busy)
        }
    }
}
