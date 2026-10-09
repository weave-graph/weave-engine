import Foundation

enum WeaveHostFailure: Error {
    case invalidResponse
    case budget
    case unavailable
    case rejected(String)
}

// Raw request and response bytes belong to the caller. Only the small control
// envelope is decoded here; graph/artifact integers never pass through NSNumber.
struct WeaveHostReply {
    let bytes: Data
    let ok: Bool
    let requiresFence: Bool
}

// Use on one owning executor. Configuration is trusted app state, separate from
// user Programs. This binding opens native SQLite; it is not a browser image host.
final class WeaveHost {
    static let requestLimit = 16 * 1024 * 1024
    static let artifactLimit = requestLimit + 4096
    static let responseLimit = 32 * 1024 * 1024 + 4096
    private let token: Data
    private var closed = false
    private var poisoned = false

    private struct Control: Decodable {
        let format: String
        let ok: Bool
        let requires_fence: Bool
        let poisoned: Bool
        let error: Failure?
        struct Failure: Decodable { let code: String }
    }
    private struct Open: Decodable {
        let value: Value?
        struct Value: Decodable { let handle: String }
    }
    private static func take(_ pointer: UnsafeMutablePointer<CChar>?) throws -> WeaveHostReply {
        guard let pointer else { throw WeaveHostFailure.invalidResponse }
        defer { weave_native_free(pointer) }
        let length = strnlen(pointer, responseLimit + 1)
        guard length <= responseLimit else { throw WeaveHostFailure.budget }
        let bytes = Data(bytes: pointer, count: length)
        let control = try JSONDecoder().decode(Control.self, from: bytes)
        guard control.format == "weave-host-response/1" else { throw WeaveHostFailure.invalidResponse }
        // No operational payload from an uncertain/poisoned instance escapes.
        if control.poisoned { throw WeaveHostFailure.rejected("E_HOST_UNCERTAIN") }
        return WeaveHostReply(bytes: bytes, ok: control.ok, requiresFence: control.requires_fence)
    }
    init(path: String, configuration: Data) throws {
        let pathBytes = Data(path.utf8)
        guard pathBytes.count <= 4096, configuration.count <= 128 * 1024 else { throw WeaveHostFailure.budget }
        let reply = try pathBytes.withUnsafeBytes { path in
            try configuration.withUnsafeBytes { config in
                try Self.take(weave_host_open(path.bindMemory(to: UInt8.self).baseAddress, path.count,
                                             config.bindMemory(to: UInt8.self).baseAddress, config.count))
            }
        }
        guard reply.ok else {
            let control = try JSONDecoder().decode(Control.self, from: reply.bytes)
            throw WeaveHostFailure.rejected(control.error?.code ?? "E_HOST_OPEN")
        }
        guard let handle = try JSONDecoder().decode(Open.self, from: reply.bytes).value?.handle,
              handle.hasPrefix("host:"), handle.utf8.count <= 32 else { throw WeaveHostFailure.invalidResponse }
        token = Data(handle.utf8)
    }
    private func invoke(_ operation: (UnsafePointer<UInt8>?, Int) throws -> UnsafeMutablePointer<CChar>?) throws -> WeaveHostReply {
        guard !closed && !poisoned else { throw WeaveHostFailure.unavailable }
        do {
            let reply = try token.withUnsafeBytes { token in
                try Self.take(operation(token.bindMemory(to: UInt8.self).baseAddress, token.count))
            }
            return reply
        } catch {
            // A malformed/uncertain response is not evidence that nothing committed.
            poisoned = true
            throw error
        }
    }
    func call(_ request: Data) throws -> WeaveHostReply {
        guard request.count <= Self.requestLimit else { throw WeaveHostFailure.budget }
        return try invoke { token, length in
            request.withUnsafeBytes { request in
                weave_host_call(token, length, request.bindMemory(to: UInt8.self).baseAddress, request.count)
            }
        }
    }
    // Explicit privileged installation; SDK bytes never supply principal/grants.
    func installHandler(sdk: Data, configuration: Data) throws -> WeaveHostReply {
        guard sdk.count <= Self.artifactLimit, configuration.count <= 256 * 1024 else { throw WeaveHostFailure.budget }
        return try invoke { token, length in
            sdk.withUnsafeBytes { sdk in
                configuration.withUnsafeBytes { config in
                    weave_host_install_handler(token, length, sdk.bindMemory(to: UInt8.self).baseAddress, sdk.count,
                                               config.bindMemory(to: UInt8.self).baseAddress, config.count)
                }
            }
        }
    }
    func setAdapterState(_ configuration: Data) throws -> WeaveHostReply {
        guard configuration.count <= 2048 else { throw WeaveHostFailure.budget }
        return try invoke { token, length in
            configuration.withUnsafeBytes { config in
                weave_host_set_adapter_state(token, length, config.bindMemory(to: UInt8.self).baseAddress, config.count)
            }
        }
    }
    static func selectArtifact(sdk: Data, selection: Data) throws -> WeaveHostReply {
        guard sdk.count <= artifactLimit, selection.count <= 1024 else { throw WeaveHostFailure.budget }
        return try sdk.withUnsafeBytes { sdk in
            try selection.withUnsafeBytes { selection in
                try take(weave_host_artifact_select(sdk.bindMemory(to: UInt8.self).baseAddress, sdk.count,
                                                   selection.bindMemory(to: UInt8.self).baseAddress, selection.count))
            }
        }
    }
    func close() throws {
        if closed { return }
        // Release even a poisoned native session; never invoke more work on it.
        defer { closed = true }
        _ = try token.withUnsafeBytes { token in
            try Self.take(weave_host_close(token.bindMemory(to: UInt8.self).baseAddress, token.count))
        }
    }
    deinit {
        if !closed {
            token.withUnsafeBytes { token in
                weave_native_free(weave_host_close(token.bindMemory(to: UInt8.self).baseAddress, token.count))
            }
        }
    }
}
