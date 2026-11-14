//! The editor fixture (#111): what the core's `parse_dsl`, `print_dsl` and `compile_workout`
//! answer for each draft text the app's tests type. The app's mock (`app/src/views/
//! workoutsBackend.ts`) answers from this file and nothing else, so it can never accept a
//! text the core refuses, or parse one differently. `CRANK_WRITE_EDITOR_FIXTURE=1` rewrites
//! it from the core (`just editor-fixture`); otherwise the test fails on any difference.
use crank_core::{Core, CoreConfig, DeviceMode, DslParse, EngineHost, Workout, WorkoutNode};
use crank_devices::sim::SimConfig;
use crank_engine::VirtualClock;
use serde_json::{Map, Value, json};
use std::{collections::HashMap, path::Path, sync::Arc};

/// Reference FTP of every draft, W (the app's library fixtures use the same).
const FTP: f64 = 250.0;

/// `(name, text)`: the drafts the app's tests and design scenes type or load.
const DRAFTS: [(&str, &str); 8] = [
    (
        "sweet-spot",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "sweet-spot-4x",
        "\"Warm-up\" 10m ramp 45-70%\n4x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "sweet-spot-4x-15m",
        "\"Warm-up\" 10m ramp 45-70%\n4x(\"Sweet spot\" 15m 90%, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "sweet-spot-no-cool-down",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)",
    ),
    ("empty", ""),
    (
        "missing-target",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m)",
    ),
    ("implausible", "\"Warm-up\" 10m ramp 45-70%\n5m 900%"),
    ("too-long", "200h 50%"),
];

const PATH: &str = "tests/fixtures/editor.json";

/// Rename every node id in tree order to `n1`, `n2`, … so the file does not depend on the
/// core's id counter; `map` records old → new for the compiled view's `nodeId`s.
fn rename(v: &mut Value, map: &mut HashMap<String, String>) {
    match v {
        Value::Array(items) => items.iter_mut().for_each(|i| rename(i, map)),
        Value::Object(o) => {
            if let Some(Value::String(id)) = o.get("id").cloned() {
                let next = format!("n{}", map.len() + 1);
                map.insert(id, next.clone());
                o.insert("id".into(), Value::String(next));
            }
            o.values_mut().for_each(|c| rename(c, map));
        }
        _ => {}
    }
}
/// Rewrite every `nodeId` through `map`.
fn remap(v: &mut Value, map: &HashMap<String, String>) {
    match v {
        Value::Array(items) => items.iter_mut().for_each(|i| remap(i, map)),
        Value::Object(o) => {
            if let Some(Value::String(id)) = o.get("nodeId").cloned() {
                o.insert("nodeId".into(), Value::String(map[&id].clone()));
            }
            o.values_mut().for_each(|c| remap(c, map));
        }
        _ => {}
    }
}

async fn entry(core: &Core, text: &str) -> Value {
    let mut out = Map::new();
    out.insert("text".into(), json!(text));
    out.insert("ftp".into(), json!(FTP));
    let nodes: Vec<WorkoutNode> = match core
        .parse_dsl(text.into(), FTP)
        .await
        .expect("parse answers")
    {
        DslParse::Invalid { problem } => {
            out.insert(
                "problem".into(),
                serde_json::to_value(problem).expect("serialisable"),
            );
            return Value::Object(out);
        }
        DslParse::Parsed { nodes } => nodes,
    };
    let printed = core.print_dsl(nodes.clone()).await.expect("print answers");
    let workout = Workout {
        id: "draft".into(),
        name: "Draft".into(),
        ftp: FTP,
        nodes: nodes.clone(),
    };
    let mut map = HashMap::new();
    let mut tree = serde_json::to_value(&nodes).expect("serialisable");
    rename(&mut tree, &mut map);
    out.insert("nodes".into(), tree);
    out.insert("printed".into(), json!(printed));
    match core.compile_workout(workout).await {
        Ok(view) => {
            let mut view = serde_json::to_value(view).expect("serialisable");
            remap(&mut view, &map);
            out.insert("compiled".into(), view);
        }
        Err(e) => {
            out.insert(
                "refused".into(),
                serde_json::to_value(e).expect("serialisable"),
            );
        }
    }
    Value::Object(out)
}

#[tokio::test]
async fn the_editor_fixture_is_the_cores_answer() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(CoreConfig {
        data_dir: dir.path().into(),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
        engine_host: EngineHost::Task,
    })
    .unwrap();
    let mut drafts = Map::new();
    for (name, text) in DRAFTS {
        drafts.insert(name.into(), entry(&core, text).await);
    }
    let want = serde_json::to_string_pretty(&Value::Object(drafts)).unwrap() + "\n";
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(PATH);
    if std::env::var_os("CRANK_WRITE_EDITOR_FIXTURE").is_some() {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "{PATH} is not the core's answer: run `just editor-fixture` and commit it"
    );
}
