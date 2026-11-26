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

/// The boxes the native library may call back into. The library keeps the `user_data` of the
/// host it accepted for as long as any call may use it (a game-file download calls back for
/// minutes or hours, and a plan, pull or push for as long as one request), and it can replace the
/// host while such a call is in flight. So a box is never freed once the library has accepted it:
/// replaced boxes move to `retired` and stay allocated (a few words and the app's two objects per
/// `SaveRelay` created, so create one for the life of the app). A refused box was never handed on
/// and is released at once.
private enum ActiveHost {
    static let lock = NSLock()
    static var box: Unmanaged<HostBox>?
    static var retired: [Unmanaged<HostBox>] = []

    /// Call `native` with `retained` under the lock that also orders the swap, so the box this
    /// process keeps as current is the one the library installed last even when several
    /// instances are created at once. `native` does not call back into the app. Returns whether
    /// the library accepted the host.
    static func install(_ retained: Unmanaged<HostBox>, accept native: () -> Bool) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard native() else {
            retained.release()
            return false
        }
        if let old = box {
            retired.append(old)
        }
        box = retained
        return true
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

/// save-relay for iOS. Keep one instance for the life of the app: the native library holds one
/// host (credentials and network) at a time, and the newest instance the library accepted owns
/// it. Check `isReady` (or `initialized`) after creating one.
///
/// Threading: every method may block on the network or the disk, so call them all off the main
/// thread. The game-file methods are meant to be used from several threads at once: run
/// `gameFilesDownload` on one background queue and call `gameFilesProgress` and
/// `gameFilesCancel` from others; they return at once. One download runs at a time per process.
/// `include/save_relay.h` is the reference for the contract (game-files version 1).
public final class SaveRelay {
    /// The game-file interface version this wrapper was written against
    /// (`save_relay_game_files_version`).
    public static let gameFilesVersion: Int32 = 1

    public let initialized: String
    /// Whether the native library accepted this instance's host. It refuses (error code "busy")
    /// while a game-file download runs; a refused instance uses the host that is already active.
    public let isReady: Bool

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
        var line = ""
        self.isReady = ActiveHost.install(retained) {
            line = home.withCString { homePointer in
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
            return line.contains("\"ok\":true")
        }
        self.initialized = line
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

    // MARK: Game files (resumable downloads; docs/assets.md, #305, #309, #310)
    //
    // They come from whichever service this device is signed in to or set up for (Dropbox,
    // Nextcloud or Google Drive under the drive.file scope). `destDir` is where an app's game files go; `stateDir` is
    // where the library keeps its checkpoints and `.part` files. Both are app-private folders on
    // the same file system, neither inside the other, neither the saves folder. Every method
    // answers with the usual JSON envelope; an error envelope also carries `error.class`
    // ("retryable", "not_yet_uploaded", "reauth_needed" or "fatal").
    //
    // `gameFilesDownload` blocks until the download finishes, fails or is cancelled (minutes to
    // hours): call it from a background queue, never the main thread, and poll
    // `gameFilesProgress` from another. One download runs at a time per process; a second call
    // answers error code "busy" (class "retryable"). The `SaveRelayNetwork` it runs over must
    // send the request's `Range` and `If-Match` headers, give back the 206 answer with its
    // `Content-Range`, `ETag` and `Dropbox-API-Result` headers unchanged, send
    // `Accept-Encoding: identity`, and stop reading after `maxResponseBytes + 1` bytes, returning
    // them (a longer answer is refused as too large, not retried). A file that changes on the
    // service mid-download is code "remote_changed" (class "retryable": the next call restarts
    // that file). A force-quit, a dead battery or a dropped network loses at most one
    // chunk; call `gameFilesDownload` again at the next launch and it continues. The library
    // does not schedule anything: the app decides when to download and when to call again.
    //
    // The instance and its credentials/network stay alive while a download runs (a method call
    // keeps `self` alive), and creating another `SaveRelay` during a download is refused.

    /// The version of the game-file interface the loaded native library speaks; compare it with
    /// `SaveRelay.gameFilesVersion`.
    public func nativeGameFilesVersion() -> Int32 {
        save_relay_game_files_version()
    }

    /// What a download would do, from file sizes only (nothing is written). Makes one network
    /// request: call it off the main thread.
    public func gameFilesPlan(appId: String?, destDir: String?, stateDir: String?) -> String {
        withOptionalCString(appId) { appPointer in
            withOptionalCString(destDir) { destPointer in
                withOptionalCString(stateDir) { statePointer in
                    takeString(save_relay_game_files_plan(appPointer, destPointer, statePointer))
                }
            }
        }
    }

    /// Download or resume everything missing. Blocks; see the note above. A cancel makes it
    /// return with `data.state` "cancelled" after the current chunk is durable.
    public func gameFilesDownload(appId: String?, destDir: String?, stateDir: String?) -> String {
        withOptionalCString(appId) { appPointer in
            withOptionalCString(destDir) { destPointer in
                withOptionalCString(stateDir) { statePointer in
                    takeString(save_relay_game_files_download(appPointer, destPointer, statePointer))
                }
            }
        }
    }

    /// The latest progress of the running or last download (`data` is null before any): `state`
    /// ("running", "complete", "cancelled" or "error"), `app_id`, `files_done`, `files_total`,
    /// `bytes_done`, `bytes_total`, `current`. Returns at once; poll about twice a second from a
    /// queue other than the one running the download.
    public func gameFilesProgress() -> String {
        takeString(save_relay_game_files_progress())
    }

    /// Stop the running download after its current chunk is durable; a later download continues.
    /// `data.running` says whether a download was running. A cancel while none runs is not
    /// remembered for the next one: if you cancel a download you have only just handed to another
    /// queue, keep your own flag and do not start it, or cancel again once `gameFilesProgress`
    /// shows it running. Returns at once.
    public func gameFilesCancel() -> String {
        takeString(save_relay_game_files_cancel())
    }

    /// Remove the checkpoints and `.part` files in `stateDir` (never the game folder). Answers
    /// "busy" while a download runs.
    public func gameFilesCleanup(stateDir: String?) -> String {
        withOptionalCString(stateDir) { statePointer in
            takeString(save_relay_game_files_cleanup(statePointer))
        }
    }

    /// Compatibility (#324): download the files of an inline manifest through the same engine and
    /// with the same rules as `gameFilesDownload` (background queue, one download at a time per
    /// process, resumable). `gameFilesCancel` stops it too (it returns with `complete` false); it reports no progress. New
    /// code uses
    /// `gameFilesDownload`.
    public func downloadGameFiles(manifestJson: String?, destDir: String?) -> String {
        withOptionalCString(manifestJson) { manifestPointer in
            withOptionalCString(destDir) { destPointer in
                takeString(save_relay_download_game_files(manifestPointer, destPointer))
            }
        }
    }

    /// Compatibility (#324): what `destDir` already holds of an inline manifest, read offline.
    public func gameFilesStatus(manifestJson: String?, destDir: String?) -> String {
        withOptionalCString(manifestJson) { manifestPointer in
            withOptionalCString(destDir) { destPointer in
                takeString(save_relay_game_files_status(manifestPointer, destPointer))
            }
        }
    }
}
