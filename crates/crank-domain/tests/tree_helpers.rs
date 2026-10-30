//! `re_id` / `find_node` parity with the frozen TS reference (`shared/src/workout/model.ts`),
//! duplicate-ID and missing-ID behaviour, bounded properties, and deep-tree safety.
use crank_domain::workout::{
    Cadence, MAX_NESTING_DEPTH, PowerTarget, RepeatNode, StepEnd, StepNode, TreeError, WorkoutNode,
    find_node, re_id,
};
use proptest::prelude::*;
use serde::Deserialize;

/// Each tree is stored once under its name; the cases refer to it (#174).
#[derive(Deserialize)]
struct Fixture {
    trees: std::collections::BTreeMap<String, Vec<WorkoutNode>>,
    #[serde(rename = "reId")]
    re_id: Vec<ReIdCase>,
    #[serde(rename = "findNode")]
    find_node: Vec<FindCase>,
}
#[derive(Deserialize)]
struct ReIdCase {
    tree: String,
    output: Vec<WorkoutNode>,
    #[serde(rename = "freshUnique")]
    fresh_unique: bool,
}
#[derive(Deserialize)]
struct FindCase {
    tree: String,
    id: String,
    output: Option<WorkoutNode>,
}

/// The crate directory at run time. `env!` would bake in the path of the build that compiled this
/// binary, which no longer exists when the Nix checks reuse it (#371); cargo and nextest set the variable.
fn manifest_dir() -> String {
    std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR for tests")
}

fn fixture() -> Fixture {
    serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/tests/fixtures/tree_helpers.json",
            manifest_dir()
        ))
        .expect("fixture is readable"),
    )
    .expect("fixture matches the domain model")
}

fn counter() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("new:{n}")
    }
}

fn id_of(n: &WorkoutNode) -> &str {
    match n {
        WorkoutNode::Step(s) => &s.id,
        WorkoutNode::Repeat(r) => &r.id,
    }
}
fn preorder<'a>(nodes: &'a [WorkoutNode], out: &mut Vec<&'a str>) {
    for n in nodes {
        out.push(id_of(n));
        if let WorkoutNode::Repeat(r) = n {
            preorder(&r.children, out);
        }
    }
}
/// Replace every ID with its kind prefix, as the fixture exporter does for the TS output.
fn kind_ids(nodes: &[WorkoutNode], prefix: Option<&str>) -> Vec<WorkoutNode> {
    nodes
        .iter()
        .map(|n| match n {
            WorkoutNode::Step(s) => WorkoutNode::Step(StepNode {
                id: prefix.unwrap_or("s").into(),
                ..s.clone()
            }),
            WorkoutNode::Repeat(r) => WorkoutNode::Repeat(RepeatNode {
                id: prefix.unwrap_or("r").into(),
                children: kind_ids(&r.children, prefix),
                ..r.clone()
            }),
        })
        .collect()
}

#[test]
fn re_id_matches_the_reference() {
    let fx = fixture();
    assert!(fx.re_id.len() >= 5);
    for case in fx.re_id {
        let input = &fx.trees[&case.tree];
        let out = re_id(input, &mut counter()).unwrap();
        assert_eq!(kind_ids(&out, None), case.output, "{}", case.tree);
        let mut ids = Vec::new();
        preorder(&out, &mut ids);
        let mut old = Vec::new();
        preorder(input, &mut old);
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        let fresh = unique.len() == ids.len() && ids.iter().all(|i| !old.contains(i));
        assert_eq!(fresh, case.fresh_unique, "{}", case.tree);
    }
}

#[test]
fn re_id_requests_ids_in_preorder() {
    let fx = fixture();
    let nested = &fx.trees["nested"];
    let out = re_id(nested, &mut counter()).unwrap();
    let mut ids = Vec::new();
    preorder(&out, &mut ids);
    assert_eq!(
        ids,
        [
            "new:1", "new:2", "new:3", "new:4", "new:5", "new:6", "new:7"
        ]
    );
}

#[test]
fn find_node_matches_the_reference() {
    let fx = fixture();
    assert!(fx.find_node.len() > 20);
    for case in fx.find_node {
        assert_eq!(
            find_node(&fx.trees[&case.tree], &case.id),
            case.output.as_ref(),
            "{}/{}",
            case.tree,
            case.id
        );
    }
}

#[test]
fn find_node_returns_the_first_match_in_depth_first_order_for_duplicates() {
    let fx = fixture();
    let dup = &fx.trees["duplicate_ids"];
    // [repeat x [step y, step x], step y, repeat x [step z]]: the repeat precedes its child x,
    // and the nested y precedes the later top-level y.
    assert!(matches!(find_node(dup, "x"), Some(WorkoutNode::Repeat(r)) if r.children.len() == 2));
    let y = find_node(dup, "y").unwrap();
    let WorkoutNode::Repeat(first) = &dup[0] else {
        unreachable!("fixture starts with a repeat")
    };
    let nested_y: &WorkoutNode = &first.children[0];
    assert!(std::ptr::eq(y, nested_y));
    assert_eq!(find_node(dup, "missing"), None);
    assert_eq!(find_node(&[], "x"), None);
}

fn step(id: String) -> WorkoutNode {
    WorkoutNode::Step(StepNode {
        id,
        label: None,
        end: StepEnd::Time { s: 60.0 },
        target: PowerTarget::Free,
        cadence: None,
    })
}
fn nest(depth: usize) -> Vec<WorkoutNode> {
    let mut nodes = vec![step("leaf".into())];
    for i in 0..depth {
        nodes = vec![WorkoutNode::Repeat(RepeatNode {
            id: format!("r{i}"),
            label: None,
            count: 1.0,
            children: nodes,
            progression: None,
            skip_last_rest: None,
        })];
    }
    nodes
}
/// Deep trees are dropped iteratively so the test's own teardown cannot overflow.
fn dispose(mut nodes: Vec<WorkoutNode>) {
    while let Some(WorkoutNode::Repeat(mut r)) = nodes.pop() {
        nodes = std::mem::take(&mut r.children);
    }
}

#[test]
fn deep_trees_do_not_panic_or_recurse_unboundedly() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            // At the bound, re_id succeeds.
            let at_bound = nest(MAX_NESTING_DEPTH);
            let out = re_id(&at_bound, &mut counter()).unwrap();
            assert_eq!(out.len(), 1);
            dispose(out);
            // Beyond the bound it is a typed error that consumes no IDs.
            for depth in [MAX_NESTING_DEPTH + 1, 100_000] {
                let deep = nest(depth);
                let mut calls = 0;
                let err = re_id(&deep, &mut || {
                    calls += 1;
                    String::new()
                })
                .unwrap_err();
                assert_eq!(err, TreeError::NestingTooDeep);
                assert_eq!(calls, 0);
                // find_node is iterative and works at any depth.
                assert!(find_node(&deep, "leaf").is_some());
                assert!(find_node(&deep, "absent").is_none());
                dispose(deep);
            }
            dispose(at_bound);
        })
        .unwrap()
        .join()
        .unwrap();
}

fn arb_tree() -> impl Strategy<Value = Vec<WorkoutNode>> {
    let leaf = "[a-c]".prop_map(|id| {
        WorkoutNode::Step(StepNode {
            id,
            label: Some("l".into()),
            end: StepEnd::Lap,
            target: PowerTarget::Ramp {
                from: 50.0,
                to: 80.0,
            },
            cadence: Some(Cadence { lo: 80.0, hi: 90.0 }),
        })
    });
    let node = leaf.prop_recursive(4, 24, 4, |inner| {
        (
            "[a-c]",
            1u32..5,
            prop::collection::vec(inner, 0..4),
            any::<bool>(),
        )
            .prop_map(|(id, count, children, skip)| {
                WorkoutNode::Repeat(RepeatNode {
                    id,
                    label: None,
                    count: f64::from(count),
                    children,
                    progression: None,
                    skip_last_rest: skip.then_some(true),
                })
            })
    });
    prop::collection::vec(node, 0..5)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn re_id_preserves_structure_and_yields_fresh_unique_ids(tree in arb_tree()) {
        let out = re_id(&tree, &mut counter()).unwrap();
        prop_assert_eq!(kind_ids(&out, Some("")), kind_ids(&tree, Some("")));
        let (mut old, mut new) = (Vec::new(), Vec::new());
        preorder(&tree, &mut old);
        preorder(&out, &mut new);
        let expected: Vec<String> = (1..=old.len()).map(|n| format!("new:{n}")).collect();
        prop_assert_eq!(new, expected.iter().map(String::as_str).collect::<Vec<_>>());
        // The input is untouched and every new node is findable.
        for id in &expected {
            prop_assert!(find_node(&out, id).is_some());
        }
    }
}
