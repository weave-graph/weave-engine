//! B4 (#40): the `compile` tool. It runs the configured compiler as a
//! sandboxed child (`weave sdk --untrusted`, plus `--read-only` for a caller
//! that can write no graph) on the caller's source and visible manifest
//! with embedding spaces, bounds it in time and output, and runs nothing.
//! The real compiler is exercised by `scripts/check_agent_compile.py` in the
//! language integration workflow; these tests use stand-in compilers.
mod common;
mod mcp_common;
use common::*;
use mcp_common::*;
use serde_json::{json, Value};
use weave_service::Limits;

#[tokio::test]
async fn compile_is_unsupported_without_a_configured_compiler() {
    let h = Harness::new();
    let t = teams(&h).await;
    assert_eq!(
        fails(&h, &t.x, "compile", json!({"source": "use G graph \"g\";"})).await,
        "E_UNSUPPORTED"
    );
    // Arguments are checked first, and carry no policy.
    for (arguments, code) in [
        (
            json!({"source": "x", "policy": {"untrusted": false}}),
            "E_MCP_INPUT",
        ),
        (json!({}), "E_MCP_INPUT"),
        (
            json!({"source": "x".repeat(weave_mcp::agent::MAX_SOURCE_BYTES + 1)}),
            "E_MCP_INPUT",
        ),
        (json!({"source": "x", "graphs": vec!["g"; 9]}), "E_BUDGET"),
    ] {
        assert_eq!(fails(&h, &t.x, "compile", arguments).await, code);
    }
}

#[cfg(unix)]
mod sandboxed {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use weave_service::compile::Compiler;

    /// A stand-in compiler: a shell script.
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn harness(binary: PathBuf, timeout: Duration, max_output_bytes: usize) -> Harness {
        Harness::with_limits(Limits {
            compiler: Some(Compiler {
                binary,
                timeout,
                max_output_bytes,
                max_memory_bytes: weave_service::compile::DEFAULT_MAX_MEMORY_BYTES,
            }),
            ..Limits::default()
        })
    }

    /// Echoes what it was given: its arguments, whether `HOME` reached it,
    /// its working directory and how many entries it holds, and the request.
    const ECHO: &str = r#"printf '{"ok":true,"args":"%s","home":"%s","cwd":"%s","entries":%s,"request":' "$*" "${HOME:-}" "$(pwd)" "$(ls -A | wc -l)"
cat
printf '}'"#;

    #[tokio::test]
    async fn the_compiler_gets_the_policy_floor_the_manifest_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let h = harness(
            script(dir.path(), "weave", ECHO),
            Duration::from_secs(10),
            1 << 20,
        );
        let t = teams(&h).await;
        let tenant = h.service.tenant("acme").unwrap();
        let head = || {
            tenant
                .with_state(|s| s.engine.head(GRAPH, "main").unwrap())
                .unwrap()
        };
        let before = head();
        // X can write: untrusted only.
        let out = ok(
            &h,
            &t.x,
            "compile",
            json!({"source": "use G graph \"x\";",
            "modules": [{"id": "m", "revision": "1", "source": "value A 1;"}]}),
        )
        .await;
        assert_eq!(
            out["policy"],
            json!({"untrusted": true, "read_only": false})
        );
        let echo = &out["response"];
        assert_eq!(echo["args"], "sdk --untrusted");
        assert_eq!(echo["home"], "", "the environment is empty");
        assert_eq!(echo["entries"], 0, "the working directory is empty");
        assert!(
            !Path::new(echo["cwd"].as_str().unwrap()).exists(),
            "and removed"
        );
        let request = &echo["request"];
        assert_eq!(request["format"], "weave-compiler-request/1");
        assert_eq!(request["source"], "use G graph \"x\";");
        assert_eq!(request["modules"][0]["id"], "m");
        let policy = &request["policy"];
        assert_eq!(policy["untrusted"], true);
        assert_eq!(policy["read_only"], false);
        let visible = &policy["visible"];
        assert_eq!(visible["format"], "weave-visible/1");
        assert_eq!(visible["principal"], "x-bot");
        assert_eq!(visible["reader_groups"], json!(["team-a"]));
        assert_eq!(visible["graphs"][0]["graph_id"], GRAPH);
        assert_eq!(visible["graphs"][0]["permission"], "write");
        // The manifest includes the graph's embedding spaces.
        assert_eq!(
            visible["graphs"][0]["embedding_spaces"],
            json!([{"id": SPACE, "revision": "1", "encoder": "test-encoder",
                "encoder_version": "1", "preprocessing": "none", "dimensions": 3,
                "metric": "cosine_distance"}])
        );
        // The manifest is the one `GET /v1/manifest` serves, plus spaces.
        let mut served = h.get("/v1/manifest", &t.x).await.json;
        served["graphs"][0]["embedding_spaces"] = visible["graphs"][0]["embedding_spaces"].clone();
        assert_eq!(&served, visible);
        // W can write nothing: read-only too, and sees no graph.
        let out = ok(&h, &t.w, "compile", json!({"source": "x"})).await;
        assert_eq!(out["policy"]["read_only"], true);
        assert_eq!(out["response"]["args"], "sdk --untrusted --read-only");
        assert_eq!(
            out["response"]["request"]["policy"]["visible"]["graphs"],
            json!([])
        );
        // `graphs` selects which graphs' spaces to list.
        let out = ok(&h, &t.x, "compile", json!({"source": "x", "graphs": []})).await;
        assert!(out["response"]["request"]["policy"]["visible"]["graphs"][0]
            .get("embedding_spaces")
            .is_none());
        // Compiling ran nothing and cited no fact.
        assert_eq!(head(), before);
        let entries = h.audit_entries(&t.root).await;
        let compiles: Vec<&Value> = entries
            .iter()
            .filter(|e| e["operation"] == "mcp.compile")
            .collect();
        assert_eq!(compiles.len(), 3);
        assert!(compiles
            .iter()
            .all(|e| e["fact_count"] == 0 && e["outcome"] == "ok"));
    }

    #[tokio::test]
    async fn diagnostics_are_results_and_misbehaving_compilers_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            // A diagnostic: exit 1, ok false.
            (
                "diagnostic",
                r#"cat > /dev/null; printf '{"ok":false,"error":{"code":"E_VISIBILITY_REQUIRED"}}'; exit 1"#,
                None,
            ),
            ("usage", "exit 2", Some("E_COMPILER")),
            (
                "mismatch",
                r#"cat > /dev/null; printf '{"ok":false}'; exit 0"#,
                Some("E_COMPILER"),
            ),
            ("not json", "cat > /dev/null; echo nope", Some("E_COMPILER")),
            (
                "too long",
                "cat > /dev/null; head -c 5000 /dev/zero | tr '\\0' 'x'",
                Some("E_COMPILER"),
            ),
            // RLIMIT_FSIZE 0: the child cannot write files.
            (
                "writes a file",
                r#"cat > /dev/null; printf x > written; printf '{"ok":true}'"#,
                Some("E_COMPILER"),
            ),
        ];
        for (name, body, code) in cases {
            let binary = script(dir.path(), &name.replace(' ', "_"), body);
            let h = harness(binary, Duration::from_secs(10), 1000);
            let t = teams(&h).await;
            match code {
                None => {
                    let out = ok(&h, &t.x, "compile", json!({"source": "x"})).await;
                    assert_eq!(out["response"]["ok"], false, "{name}");
                    assert_eq!(out["response"]["error"]["code"], "E_VISIBILITY_REQUIRED");
                }
                Some(code) => {
                    assert_eq!(
                        fails(&h, &t.x, "compile", json!({"source": "x"})).await,
                        code,
                        "{name}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn a_compiler_that_runs_too_long_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let h = harness(
            script(dir.path(), "slow", "sleep 30"),
            Duration::from_millis(300),
            1000,
        );
        let t = teams(&h).await;
        let started = Instant::now();
        assert_eq!(
            fails(&h, &t.x, "compile", json!({"source": "x"})).await,
            "E_COMPILER"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        // The tenant serves the next request.
        ok(&h, &t.x, "describe_schema", json!({})).await;
    }
}
