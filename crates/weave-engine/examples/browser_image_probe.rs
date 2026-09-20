//! First toolchain/image smoke only; IndexedDB fences are tested by the later browser host.
use serde_json::json;
use sha2::{Digest, Sha256};
use weave_contract::Program;
use weave_engine::{Engine, HostContext};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("weave-image-probe-{}", std::process::id()));
    std::fs::create_dir(&dir)?;
    let outcome = run(&dir);
    std::fs::remove_dir_all(&dir)?;
    outcome
}

fn run(dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::open_single_owner_image(dir.join("original.sqlite"))?;
    let initial_image = engine.export_single_owner_image()?;
    let program: Program = serde_json::from_str(
        r#"{"version":"0.18.0","commands":[{"op":"commit","graph_id":"browser","data":{"nodes":[{"id":"n","entity_id":"E","space_id":"s","properties":{"exact":9007199254740993,"max":9223372036854775807,"min":-9223372036854775808}}]}}]}"#,
    )?;
    let host = HostContext::new("owner", ["browser".into()]);
    let result = engine.execute(&program, &host)?;
    let image = engine.export_single_owner_image()?;
    assert_ne!(initial_image, image);
    std::fs::write(dir.join("restored.sqlite"), &image)?;
    let mut restored = Engine::open_single_owner_image(dir.join("restored.sqlite"))?;
    let query: Program = serde_json::from_value(json!({"version":"0.18.0","commands":[{
        "op":"query","query":{"graph_id":"browser"}
    }]}))?;
    let original_result = serde_json::to_vec(&engine.execute(&query, &host)?)?;
    let restored_result = serde_json::to_vec(&restored.execute(&query, &host)?)?;
    assert_eq!(original_result, restored_result);
    assert!(
        std::str::from_utf8(&restored_result)?.contains("9007199254740993"),
        "{}",
        std::str::from_utf8(&restored_result)?
    );
    assert_eq!(
        engine.head("browser", "main")?,
        restored.head("browser", "main")?
    );
    assert_eq!(engine.event_count()?, restored.event_count()?);
    let reopened_image = restored.export_single_owner_image()?;
    let changed_offsets: Vec<usize> = image
        .iter()
        .zip(&reopened_image)
        .enumerate()
        .filter_map(|(i, (a, b))| (a != b).then_some(i))
        .collect();
    assert_eq!(image.len(), reopened_image.len());
    assert!(
        changed_offsets
            .iter()
            .all(|i| (24..28).contains(i) || (92..96).contains(i)),
        "unexpected changed offsets: {:?}",
        changed_offsets
    );
    println!(
        "{}",
        json!({
            "profile":"experimental-image-smoke-v1","image_bytes":image.len(),
            "image_sha256":format!("{:x}",Sha256::digest(&image)),
            "restored_query_identical":true,"exact_i64":true,"commit_results":result,
            "query_json":std::str::from_utf8(&restored_result)?,
            "reopen_header_change_offsets":changed_offsets,"indexeddb_durability_tested":false
        })
    );
    Ok(())
}
