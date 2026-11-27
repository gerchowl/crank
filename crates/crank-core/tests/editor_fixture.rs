//! The editor fixture (#111): what the core's `parse_dsl`, `print_dsl` and `compile_workout`
//! answer for each draft text the app's tests type. The app's mock (`app/src/views/
//! workoutsBackend.ts`) answers from this file and nothing else, so it can never accept a
//! text the core refuses, or parse one differently. `CRANK_WRITE_EDITOR_FIXTURE=1` rewrites
//! it from the core (`just editor-fixture`); otherwise the test fails on any difference.
use crank_core::{
    Core, CoreConfig, DeviceMode, DslParse, EngineHost, VarDecl, Workout, WorkoutNode,
};
use crank_devices::sim::SimConfig;
use crank_engine::VirtualClock;
use serde_json::{Map, Value, json};
use std::{collections::HashMap, path::Path, sync::Arc};

/// Reference FTP of every draft, W (the app's library fixtures use the same).
const FTP: f64 = 250.0;

/// `(name, text)`: the drafts the app's tests and design scenes type or load.
const DRAFTS: [(&str, &str); 23] = [
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
    // The outline's edits of `sweet-spot` (#653), each as the core prints it.
    (
        "dup-warm-up",
        "\"Warm-up\" 10m ramp 45-70%\n\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "add-step",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%\n5m 50%",
    ),
    (
        "wrap-cool-down",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)\n2x(\"Cool-down\" 5m ramp 60-40%)",
    ),
    (
        "unwrapped",
        "\"Warm-up\" 10m ramp 45-70%\n\"Sweet spot\" 12m 90%\n\"Rest\" 4m 55%\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "cool-down-up",
        "\"Warm-up\" 10m ramp 45-70%\n\"Cool-down\" 5m ramp 60-40%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)",
    ),
    (
        "sweet-spot-95",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m 95%, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "warm-up-50-75",
        "\"Warm-up\" 10m ramp 50-75%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "rest-recover",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m 90%, \"Recover\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    // The outline's first step in a new workout.
    ("one-step", "5m 50%"),
    // A duration of 0.1 min, rounded to whole seconds by the outline's form.
    (
        "warm-up-6s",
        "\"Warm-up\" 6s ramp 45-70%\n3x(\"Sweet spot\" 12m 90%, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    // Moves and deletes inside the repeat; deleting its last child removes the repeat.
    (
        "rest-up",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Rest\" 4m 55%, \"Sweet spot\" 12m 90%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "no-sweet-spot",
        "\"Warm-up\" 10m ramp 45-70%\n3x(\"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "no-repeat",
        "\"Warm-up\" 10m ramp 45-70%\n\"Cool-down\" 5m ramp 60-40%",
    ),
    // Declarations (ADR-0008 §2.6): `$spare` is unused and kept by an outline edit.
    (
        "with-vars",
        "$spare = 5w\n$sst = 90%\n\n\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m $sst, \"Rest\" 4m 55%)\n\"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "with-vars-no-cool-down",
        "$spare = 5w\n$sst = 90%\n\n\"Warm-up\" 10m ramp 45-70%\n3x(\"Sweet spot\" 12m $sst, \"Rest\" 4m 55%)",
    ),
];

/// `(name, draft, pointer, value)`: trees no text gives, the outline's refused edits (#653): the
/// draft's tree with the JSON value at `pointer` replaced. The entry's text is the tree's printed
/// text and carries the core's answer for parsing it back (a `problem` when it does not).
const TREES: [(&str, &str, &str, f64); 2] = [
    ("zero-duration", "sweet-spot", "/0/end/s", 0.0),
    (
        "negative-power",
        "sweet-spot",
        "/1/children/0/target/value",
        -10.0,
    ),
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
    match core
        .parse_dsl(text.into(), FTP)
        .await
        .expect("parse answers")
    {
        DslParse::Invalid { problem } => {
            out.insert(
                "problem".into(),
                serde_json::to_value(problem).expect("serialisable"),
            );
        }
        DslParse::Parsed { nodes, vars } => tree(core, vars, nodes, &mut out).await,
    }
    Value::Object(out)
}

/// A tree no text gives (`TREES`): its printed text, the core's answer for parsing that text back
/// (a `problem`, or nothing when it parses), then the tree's own answers as a draft's.
async fn tree_entry(core: &Core, draft: &str, pointer: &str, value: f64) -> Value {
    let (_, text) = DRAFTS.iter().find(|(n, _)| *n == draft).expect("a draft");
    let DslParse::Parsed { nodes, vars } = core
        .parse_dsl((*text).into(), FTP)
        .await
        .expect("parse answers")
    else {
        panic!("{draft} parses");
    };
    let mut v = serde_json::to_value(nodes).expect("serialisable");
    *v.pointer_mut(pointer).expect("the pointer names a value") = json!(value);
    let nodes: Vec<WorkoutNode> = serde_json::from_value(v).expect("a tree");
    let printed = core
        .print_dsl(vars.clone(), nodes.clone())
        .await
        .expect("print answers");
    let mut out = Map::new();
    out.insert("text".into(), json!(printed));
    out.insert("ftp".into(), json!(FTP));
    if let DslParse::Invalid { problem } =
        core.parse_dsl(printed, FTP).await.expect("parse answers")
    {
        out.insert(
            "problem".into(),
            serde_json::to_value(problem).expect("serialisable"),
        );
    }
    tree(core, vars, nodes, &mut out).await;
    Value::Object(out)
}

/// A tree's answers: its declarations, its nodes (ids renamed), its printed text, and its
/// compiled view or refusal.
async fn tree(
    core: &Core,
    vars: Vec<VarDecl>,
    nodes: Vec<WorkoutNode>,
    out: &mut Map<String, Value>,
) {
    let printed = core
        .print_dsl(vars.clone(), nodes.clone())
        .await
        .expect("print answers");
    out.insert(
        "vars".into(),
        serde_json::to_value(&vars).expect("serialisable"),
    );
    let workout = Workout {
        id: "draft".into(),
        name: "Draft".into(),
        ftp: FTP,
        nodes: nodes.clone(),
        vars,
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
    for (name, draft, pointer, value) in TREES {
        drafts.insert(name.into(), tree_entry(&core, draft, pointer, value).await);
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
