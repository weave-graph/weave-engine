import Foundation

enum WeaveFailure: Error { case invalidResponse; case rejected(String) }
// The embedding app owns the host grants. Graph source never supplies this initializer's authority.
final class Weave {
    private let handle: UInt64
    private var closed = false
    let contract: String
    private static func decode(_ pointer: UnsafeMutablePointer<CChar>?) throws -> [String: Any] {
        guard let pointer else { throw WeaveFailure.invalidResponse }
        defer { weave_native_free(pointer) }
        let data = Data(String(cString: pointer).utf8)
        guard let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw WeaveFailure.invalidResponse }
        guard envelope["ok"] as? Bool == true, let value = envelope["value"] as? [String: Any] else {
            throw WeaveFailure.rejected((envelope["error"] as? [String: Any])?["code"] as? String ?? "E_NATIVE_RESPONSE")
        }
        if let error = value["execution_error"] as? [String: Any] { throw WeaveFailure.rejected(error["code"] as? String ?? "E_EXECUTION") }
        return value
    }
    init(path: String, principal: String, writableGraphs: [String]) throws {
        let pathData = Array(path.utf8)
        let authority = try JSONSerialization.data(withJSONObject: ["principal": principal, "writable_graphs": writableGraphs])
        let value = try pathData.withUnsafeBufferPointer { path in
            try authority.withUnsafeBytes { host in
                try Self.decode(weave_native_open(path.baseAddress, path.count, host.bindMemory(to: UInt8.self).baseAddress, host.count))
            }
        }
        guard let number = value["handle"] as? NSNumber, let version = value["contract"] as? String else { throw WeaveFailure.invalidResponse }
        handle = number.uint64Value
        contract = version
    }
    // Call on the owning executor. The C ABI also serializes engine operations internally.
    func execute(_ commands: [[String: Any]]) throws -> [[String: Any]] {
        guard !closed else { throw WeaveFailure.rejected("E_NATIVE_HANDLE") }
        let program = try JSONSerialization.data(withJSONObject: ["version": contract, "commands": commands])
        let value = try program.withUnsafeBytes { bytes in
            try Self.decode(weave_native_execute(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count))
        }
        guard let results = value["results"] as? [[String: Any]] else { throw WeaveFailure.invalidResponse }
        return results
    }
    func close() throws {
        if !closed { _ = try Self.decode(weave_native_close(handle)); closed = true }
    }
    deinit { if !closed { weave_native_free(weave_native_close(handle)) } }
}
