import Foundation
#if canImport(UIKit)
import UIKit
#endif

func runProbe(directory: URL, stage: String) throws -> [String: Any] {
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let database = directory.appendingPathComponent("weave.sqlite").path
    let engine = try Weave(path: database, principal: "alice", writableGraphs: ["mobile", "marker"])
    defer { try? engine.close() }
    func read(_ actor: String = "alice") throws -> [String: Any] {
        let reader = try Weave(path: database, principal: actor, writableGraphs: [])
        defer { try? reader.close() }
        let result = try reader.execute([["op": "query", "query": ["graph_id": "mobile"]]])
        guard let value = result.first?["result"] as? [String: Any], let graph = value["graph"] as? [String: Any] else { throw WeaveFailure.invalidResponse }
        return graph
    }
    if stage == "seed" {
        let data: [String: Any] = ["schema": ["id": "mobile-schema", "revision": "1", "nodes": ["Record": ["properties": ["exact": ["value_type": "decimal", "required": true]]]], "edges": [:]], "nodes": [["id": "public", "entity_id": "Public", "space_id": "offline", "type_id": "Record", "properties": ["exact": "9007199254740993"]], ["id": "private", "entity_id": "Private", "space_id": "offline", "type_id": "Record", "properties": ["exact": "0.3"], "readers": ["alice"]]], "edges": []]
        let result = try engine.execute([["op": "commit", "graph_id": "mobile", "data": data]])
        guard let revision = result.first?["revision"] as? String else { throw WeaveFailure.invalidResponse }
        try Data(revision.utf8).write(to: directory.appendingPathComponent("revision.txt"), options: .atomic)
        return ["stage": stage, "contract": engine.contract, "seeded": true]
    }
    let alice = try read()
    let bob = try read("bob")
    guard let nodes = alice["nodes"] as? [[String: Any]], nodes.count == 2,
          let publicNode = nodes.first(where: { $0["id"] as? String == "public" }),
          (publicNode["properties"] as? [String: Any])?["exact"] as? String == "9007199254740993",
          (bob["nodes"] as? [[String: Any]])?.count == 1 else { throw WeaveFailure.rejected("E_PROBE_PRIVACY") }
    let revision = try String(contentsOf: directory.appendingPathComponent("revision.txt"), encoding: .utf8)
    let pinned = try engine.execute([["op": "query", "query": ["graph_id": "mobile", "revision": revision]]])
    guard let pinnedValue = pinned.first?["result"] as? [String: Any], let pinnedGraph = pinnedValue["graph"] as? [String: Any], (pinnedGraph["nodes"] as? [Any])?.count == 2 else { throw WeaveFailure.invalidResponse }
    if stage == "rollback" {
        do {
            _ = try engine.execute([["op": "commit", "graph_id": "marker", "data": ["nodes": [], "edges": []]], ["op": "commit", "graph_id": "mobile", "expected_head": "stale", "data": ["nodes": [], "edges": []]]])
            throw WeaveFailure.rejected("E_PROBE_EXPECTED_REJECTION")
        } catch WeaveFailure.rejected("E_CONFLICT") { }
        _ = try engine.execute([["op": "commit", "graph_id": "marker", "data": ["nodes": [], "edges": []]]])
    }
    return ["stage": stage, "contract": engine.contract, "restarted_persistence": true, "exact_decimal": "9007199254740993", "alice_nodes": 2, "bob_nodes": 1, "pinned_history": true, "rollback": stage == "rollback"]
}

#if canImport(UIKit)
@main final class ProbeDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?
    func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = UIViewController()
        window?.makeKeyAndVisible()
        DispatchQueue.main.async {
            let stage = ProcessInfo.processInfo.arguments.last ?? "read"
            let directory = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            do {
                let value = try runProbe(directory: directory, stage: stage)
                let result = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
                try result.write(to: directory.appendingPathComponent("result-\(stage).json"), options: .atomic)
                print(String(decoding: result, as: UTF8.self)); exit(0)
            } catch {
                let data = Data("\(error)".utf8)
                try? data.write(to: directory.appendingPathComponent("error-\(stage).txt"))
                print(error); exit(2)
            }
        }
        return true
    }
}
#else
@main enum Probe {
    static func main() throws {
        guard CommandLine.arguments.count == 3 else { throw WeaveFailure.invalidResponse }
        let result = try runProbe(directory: URL(fileURLWithPath: CommandLine.arguments[1]), stage: CommandLine.arguments[2])
        print(String(decoding: try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]), as: UTF8.self))
    }
}
#endif
