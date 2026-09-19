use weave_contract::Program;
use weave_engine::{Capsule, Engine, HostContext};
fn main() {
    let host = HostContext::new("fuzz", ["fuzz".into()]);
    let mut engine = Engine::memory().unwrap();
    let plan: Program =
        serde_json::from_str(include_str!("../seeds/program_atomic/geometry.json")).unwrap();
    let output = engine
        .execute(&plan, &host)
        .expect("geometry seed executes full graph workflow");
    assert_eq!(output.len(), plan.commands.len());
    let capsule: Capsule =
        serde_json::from_str(include_str!("../seeds/capsule_receive/empty-graph.json")).unwrap();
    let mut receiver = Engine::memory().unwrap();
    assert_eq!(receiver.receive_capsule(&capsule, &host).unwrap(), 1);
    assert_eq!(receiver.receive_capsule(&capsule, &host).unwrap(), 0);
    assert!(receiver.events().unwrap().is_empty());
    println!("Valid seeds exercise full geometry execution and idempotent capsule import");
}
