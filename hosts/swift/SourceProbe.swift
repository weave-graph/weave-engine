// Test application. Fixture files are supplied by the acceptance controller;
// WeaveHost contains no domain graph names, recipes or test keys.
import Foundation
#if canImport(UIKit)
import UIKit
#endif

func sourceProbe(directory: URL, resources: URL, stage: String) throws -> [String: Any] {
    let fm = FileManager.default
    try fm.createDirectory(at: directory, withIntermediateDirectories: true)
    func fixture(_ name: String) throws -> Data { try Data(contentsOf: resources.appendingPathComponent(name)) }
    let config = try JSONSerialization.jsonObject(with: fixture("fixture.json")) as! [String: Any]
    let adapter = (config["adapters"] as! [String: String])["diagnostic"]!
    let host = try WeaveHost(path: directory.appendingPathComponent("engine.sqlite").path,
                             configuration: fixture(stage == "reviewer" || stage == "foreign" ? "reviewer.json" : "authority.json"))
    defer { try? host.close() }
    var operations = 0
    func save(_ reply: WeaveHostReply, _ name: String, expectedOK: Bool = true) throws -> Data {
        guard reply.ok == expectedOK else {
            let envelope = try JSONSerialization.jsonObject(with: reply.bytes) as! [String: Any]
            throw WeaveHostFailure.rejected((envelope["error"] as? [String: Any])?["code"] as? String ?? "E_PROBE_EXPECTED_OUTCOME")
        }
        try reply.bytes.write(to: directory.appendingPathComponent(stage + "-" + name + ".json"), options: .atomic)
        operations += 1
        return reply.bytes
    }
    func command(_ fields: [String: Any]) throws -> Data {
        // Only control strings are constructed here, never Program/artifact bodies.
        try JSONSerialization.data(withJSONObject: ["format": "weave-host-request/1", "operation": fields])
    }
    func value(_ raw: Data) throws -> [String: Any] {
        // Read only occurrence/lease/preparation strings. Do not serialize bodies.
        let envelope = try JSONSerialization.jsonObject(with: raw) as! [String: Any]
        return envelope["value"] as! [String: Any]
    }
    func pending() throws -> [String: Any] { try value(Data(contentsOf: directory.appendingPathComponent("poll-event.json"))) }
    func operation(_ kind: String) throws -> [String: Any] {
        let event = try pending()
        var op: [String: Any] = ["kind": kind, "adapter": adapter, "event": event["id"]!, "lease": event["lease"]!]
        if kind == "complete" {
            let prepared = try value(Data(contentsOf: directory.appendingPathComponent("prepare-preparation.json")))
            op["preparation"] = prepared["preparation_id"]!
        }
        return op
    }
    switch stage {
    case "seed":
        let sdk = try fixture("handler-sdk.json")
        _ = try save(WeaveHost.selectArtifact(sdk: sdk, selection: Data("{\"kind\":\"original\"}".utf8)), "inventory")
        _ = try save(host.call(fixture("seed-request.json")), "seed")
        _ = try save(host.installHandler(sdk: sdk, configuration: fixture("install.json")), "install")
        _ = try save(host.setAdapterState(JSONSerialization.data(withJSONObject: ["adapter": adapter, "state": "running"])), "running")
        let event = try value(save(host.call(command(["kind": "poll", "adapter": adapter])), "initial-event"))
        let args: [String: Any] = ["kind": "prepare", "adapter": adapter, "event": event["id"]!, "lease": event["lease"]!]
        let prepared = try value(save(host.call(command(args)), "initial-preparation"))
        _ = try save(host.call(command(["kind": "complete", "adapter": adapter, "event": event["id"]!, "lease": event["lease"]!, "preparation": prepared["preparation_id"]!])), "initial-completion")
    case "offline", "offline-duplicate":
        _ = try save(host.call(fixture("offline-request.json")), "offline")
    case "poll":
        _ = try save(host.call(command(["kind": "poll", "adapter": adapter])), "event")
    case "prepare-lost", "complete-lost":
        let kind = stage == "prepare-lost" ? "prepare" : "complete"
        let reply = try host.call(command(operation(kind)))
        guard reply.ok else { throw WeaveHostFailure.rejected("E_PROBE_LOST_OPERATION") }
        // SQLite committed; no preparation/completion payload or report is released.
        _exit(92)
    case "prepare":
        _ = try save(host.call(command(operation("prepare"))), "preparation")
    case "complete":
        _ = try save(host.call(command(operation("complete"))), "completion")
    case "read":
        for name in ["old-installation", "current-installation", "warnings"] {
            _ = try save(host.call(fixture(name + "-request.json")), name)
        }
    case "reviewer":
        _ = try save(host.call(fixture("warnings-request.json")), "warnings")
    case "foreign":
        _ = try save(host.call(command(operation("prepare"))), "denial", expectedOK: false)
    case "rollback":
        _ = try save(host.call(fixture("rollback-request.json")), "denial", expectedOK: false)
    default: throw WeaveHostFailure.rejected("E_PROBE_STAGE")
    }
    return ["format": "weave-ios-source-probe/1", "stage": stage, "operations": operations, "status": "passed"]
}

#if canImport(UIKit)
@main final class SourceProbeDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?
    func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = UIViewController()
        window?.makeKeyAndVisible()
        DispatchQueue.main.async {
            let stage = ProcessInfo.processInfo.arguments.last ?? "read"
            let directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("WeaveSourceProbe")
            do {
                let value = try sourceProbe(directory: directory, resources: Bundle.main.resourceURL!, stage: stage)
                let bytes = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
                try bytes.write(to: directory.appendingPathComponent(stage + "-report.json"), options: .atomic)
                print(String(decoding: bytes, as: UTF8.self)); fflush(stdout); exit(0)
            } catch { print("Weave source probe: \(error)"); fflush(stdout); exit(2) }
        }
        return true
    }
}
#else
@main enum SourceProbe {
    static func main() throws {
        guard CommandLine.arguments.count == 4 else { throw WeaveHostFailure.invalidResponse }
        let result = try sourceProbe(directory: URL(fileURLWithPath: CommandLine.arguments[1]), resources: URL(fileURLWithPath: CommandLine.arguments[2]), stage: CommandLine.arguments[3])
        print(String(decoding: try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]), as: UTF8.self))
    }
}
#endif
