import Foundation
import SaveRelayC

public struct SaveRelayRequest {
    public let method: String
    public let url: String
    public let headers: String
    public let body: Data
    public let maxResponseBytes: Int

    public init(
        method: String,
        url: String,
        headers: String,
        body: Data,
        maxResponseBytes: Int
    ) {
        self.method = method
        self.url = url
        self.headers = headers
        self.body = body
        self.maxResponseBytes = maxResponseBytes
    }
}

public struct SaveRelayResponse {
    public let status: UInt16
    public let headers: String?
    public let body: Data

    public init(status: UInt16, headers: String? = nil, body: Data = Data()) {
        self.status = status
        self.headers = headers
        self.body = body
    }
}

public protocol SaveRelayCredentials: AnyObject {
    func secret(for account: String) -> Data?
    func setSecret(_ secret: String, for account: String) -> Bool
    func deleteSecret(for account: String) -> Bool
}

public protocol SaveRelayNetwork: AnyObject {
    func respond(to request: SaveRelayRequest) -> SaveRelayResponse?
}

private final class HostBox {
    let credentials: SaveRelayCredentials
    let network: SaveRelayNetwork

    init(credentials: SaveRelayCredentials, network: SaveRelayNetwork) {
        self.credentials = credentials
        self.network = network
    }
}

private func takeString(_ pointer: UnsafeMutablePointer<CChar>!) -> String {
    guard let pointer = pointer else { return "" }
    defer { save_relay_free(pointer) }
    return String(cString: pointer)
}

private func withOptionalCString<T>(
    _ value: String?,
    _ body: (UnsafePointer<CChar>?) -> T
) -> T {
    guard let value = value else { return body(nil) }
    return value.withCString { body($0) }
}

private func hostCredentialGet(
    _ userData: UnsafeMutableRawPointer!,
    _ account: UnsafePointer<CChar>!,
    _ buf: UnsafeMutablePointer<UInt8>!,
    _ cap: Int,
    _ len: UnsafeMutablePointer<Int>!
) -> Int32 {
    guard let userData = userData, let account = account, let buf = buf,
          let len = len else { return 2 }
    let box = Unmanaged<HostBox>.fromOpaque(userData).takeUnretainedValue()
    guard let secret = box.credentials.secret(for: String(cString: account))
    else { return 1 }
    let bytes = [UInt8](secret)
    guard bytes.count <= cap else { return 2 }
    if !bytes.isEmpty {
        bytes.withUnsafeBufferPointer {
            buf.update(from: $0.baseAddress!, count: bytes.count)
        }
    }
    len.pointee = bytes.count
    return 0
}

private func hostCredentialSet(
    _ userData: UnsafeMutableRawPointer!,
    _ account: UnsafePointer<CChar>!,
    _ secret: UnsafePointer<CChar>!
) -> Int32 {
    guard let userData = userData, let account = account, let secret = secret
    else { return 2 }
    let box = Unmanaged<HostBox>.fromOpaque(userData).takeUnretainedValue()
    let stored = box.credentials.setSecret(
        String(cString: secret),
        for: String(cString: account)
    )
    return stored ? 0 : 2
}

private func hostCredentialDelete(
    _ userData: UnsafeMutableRawPointer!,
    _ account: UnsafePointer<CChar>!
) -> Int32 {
    guard let userData = userData, let account = account else { return 2 }
    let box = Unmanaged<HostBox>.fromOpaque(userData).takeUnretainedValue()
    return box.credentials.deleteSecret(for: String(cString: account)) ? 0 : 2
}

private func hostHttp(
    _ userData: UnsafeMutableRawPointer!,
    _ request: UnsafePointer<SaveRelayHttpRequest>!,
    _ sink: OpaquePointer!
) -> Int32 {
    guard let userData = userData, let request = request, let sink = sink
    else { return 2 }
    let box = Unmanaged<HostBox>.fromOpaque(userData).takeUnretainedValue()
    let raw = request.pointee
    let method = String(cString: raw.method)
    let url = String(cString: raw.url)
    let headers = raw.headers.map { String(cString: $0) } ?? ""
    let body: Data
    if raw.body_len > 0, let pointer = raw.body {
        body = Data(bytes: pointer, count: raw.body_len)
    } else {
        body = Data()
    }
    let outgoing = SaveRelayRequest(
        method: method,
        url: url,
        headers: headers,
        body: body,
        maxResponseBytes: raw.max_response_bytes
    )
    guard let answer = box.network.respond(to: outgoing) else { return 1 }
    let headerText = answer.headers ?? ""
    let bodyBytes = [UInt8](answer.body)
    var result: Int32 = 1
    headerText.withCString { headerPointer in
        bodyBytes.withUnsafeBufferPointer { bodyPointer in
            result = save_relay_http_respond(
                sink,
                answer.status,
                headerPointer,
                bodyPointer.baseAddress,
                bodyBytes.count
            )
        }
    }
    return result == 0 ? 0 : 1
}

public final class SaveRelay {
    public let initialized: String
    private let retainedHost: Unmanaged<HostBox>

    public init(
        home: String,
        data: String,
        config: String,
        documents: String? = nil,
        credentials: SaveRelayCredentials,
        network: SaveRelayNetwork
    ) {
        let box = HostBox(credentials: credentials, network: network)
        let retained = Unmanaged.passRetained(box)
        self.retainedHost = retained
        self.initialized = home.withCString { homePointer in
            data.withCString { dataPointer in
                config.withCString { configPointer in
                    withOptionalCString(documents) { documentsPointer in
                        var host = SaveRelayHost(
                            user_data: retained.toOpaque(),
                            home: homePointer,
                            data: dataPointer,
                            config: configPointer,
                            documents: documentsPointer,
                            cred_get: hostCredentialGet,
                            cred_set: hostCredentialSet,
                            cred_delete: hostCredentialDelete,
                            http: hostHttp
                        )
                        return takeString(save_relay_init(&host))
                    }
                }
            }
        }
    }

    deinit {
        retainedHost.release()
    }

    public func pull(
        manifestJson: String?,
        savesDir: String?,
        resolve: String? = nil
    ) -> String {
        withOptionalCString(manifestJson) { manifestPointer in
            withOptionalCString(savesDir) { savesPointer in
                withOptionalCString(resolve) { resolvePointer in
                    takeString(
                        save_relay_pull(
                            manifestPointer,
                            savesPointer,
                            resolvePointer
                        )
                    )
                }
            }
        }
    }

    public func push(
        manifestJson: String?,
        savesDir: String?,
        resolve: String? = nil
    ) -> String {
        withOptionalCString(manifestJson) { manifestPointer in
            withOptionalCString(savesDir) { savesPointer in
                withOptionalCString(resolve) { resolvePointer in
                    takeString(
                        save_relay_push(
                            manifestPointer,
                            savesPointer,
                            resolvePointer
                        )
                    )
                }
            }
        }
    }

    public func status(manifestJson: String?, savesDir: String?) -> String {
        withOptionalCString(manifestJson) { manifestPointer in
            withOptionalCString(savesDir) { savesPointer in
                takeString(save_relay_status(manifestPointer, savesPointer))
            }
        }
    }

    public func setup(
        server: String?,
        user: String?,
        password: String?,
        device: String? = nil,
        root: String? = nil
    ) -> String {
        withOptionalCString(server) { serverPointer in
            withOptionalCString(user) { userPointer in
                withOptionalCString(password) { passwordPointer in
                    withOptionalCString(device) { devicePointer in
                        withOptionalCString(root) { rootPointer in
                            takeString(
                                save_relay_setup(
                                    serverPointer,
                                    userPointer,
                                    passwordPointer,
                                    devicePointer,
                                    rootPointer
                                )
                            )
                        }
                    }
                }
            }
        }
    }

    public func signInStart(
        service: String?,
        redirectUri: String?,
        device: String? = nil,
        root: String? = nil
    ) -> String {
        withOptionalCString(service) { servicePointer in
            withOptionalCString(redirectUri) { redirectPointer in
                withOptionalCString(device) { devicePointer in
                    withOptionalCString(root) { rootPointer in
                        takeString(
                            save_relay_sign_in_start(
                                servicePointer,
                                redirectPointer,
                                devicePointer,
                                rootPointer
                            )
                        )
                    }
                }
            }
        }
    }

    public func signInFinish(redirectUrl: String?) -> String {
        withOptionalCString(redirectUrl) { redirectPointer in
            takeString(save_relay_sign_in_finish(redirectPointer))
        }
    }

    // MARK: Game files (resumable downloads from Dropbox; docs/assets.md, #305)
    //
    // `destDir` is where an app's game files go; `stateDir` is where the library keeps its
    // checkpoints and `.part` files. Both are app-private folders on the same file system,
    // neither inside the other, neither the saves folder. All five answer with the usual JSON
    // envelope; an error envelope also carries `error.class` ("retryable", "not_yet_uploaded",
    // "reauth_needed" or "fatal").
    //
    // `gameFilesDownload` blocks until the download finishes, fails or is cancelled: call it
    // from a background queue, never the main thread, and poll `gameFilesProgress` from
    // another. The `SaveRelayNetwork` it runs over must send the request's `Range` header, give
    // back the 206 answer with its `Content-Range` and `Dropbox-API-Result` headers unchanged,
    // send `Accept-Encoding: identity`, and stop at `maxResponseBytes`. A force-quit, a dead
    // battery or a dropped network loses at most one chunk; call `gameFilesDownload` again at
    // the next launch and it continues.

    /// What a download would do, from file sizes only (nothing is written).
    public func gameFilesPlan(appId: String?, destDir: String?, stateDir: String?) -> String {
        withOptionalCString(appId) { appPointer in
            withOptionalCString(destDir) { destPointer in
                withOptionalCString(stateDir) { statePointer in
                    takeString(save_relay_game_files_plan(appPointer, destPointer, statePointer))
                }
            }
        }
    }

    /// Download or resume everything missing. Blocks; see the note above.
    public func gameFilesDownload(appId: String?, destDir: String?, stateDir: String?) -> String {
        withOptionalCString(appId) { appPointer in
            withOptionalCString(destDir) { destPointer in
                withOptionalCString(stateDir) { statePointer in
                    takeString(save_relay_game_files_download(appPointer, destPointer, statePointer))
                }
            }
        }
    }

    /// The latest progress of the running or last download (`data` is null before any).
    public func gameFilesProgress() -> String {
        takeString(save_relay_game_files_progress())
    }

    /// Stop the running download after its current chunk is durable; a later download continues.
    public func gameFilesCancel() -> String {
        takeString(save_relay_game_files_cancel())
    }

    /// Remove the checkpoints and `.part` files in `stateDir` (never the game folder).
    public func gameFilesCleanup(stateDir: String?) -> String {
        withOptionalCString(stateDir) { statePointer in
            takeString(save_relay_game_files_cleanup(statePointer))
        }
    }
}
