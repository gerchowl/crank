//! Reusable schema-based golden-vector comparison (ADR-0001 §3).
use serde_json::Value;

/// The crate directory at run time. `env!` would bake in the path of the build that compiled this
/// binary, which no longer exists when the Nix checks reuse it (#371); cargo and nextest set the variable.
fn manifest_dir() -> String {
    std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR for tests")
}

pub fn load(area: &str) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(format!("{}/tests/vectors/{area}.json", manifest_dir()))
            .expect("valid vector schema and fixture"),
    )
    .expect("valid vector schema and fixture")
}
pub fn number(v: &Value) -> f64 {
    match v.as_str() {
        Some("NaN") => f64::NAN,
        Some("Infinity") => f64::INFINITY,
        Some("-Infinity") => f64::NEG_INFINITY,
        _ => v.as_f64().expect("valid vector schema and fixture"),
    }
}
pub fn numbers(v: &Value) -> Vec<f64> {
    v.as_array()
        .expect("valid vector schema and fixture")
        .iter()
        .map(number)
        .collect()
}

/// A scalar node is the string shorthand, `{"kind": ..}` (with modifiers such as `nullable`), or
/// `{"number": "float"}` for a node shared with object/array variants.
fn scalar_kind(schema: &Value) -> Option<&str> {
    schema
        .as_str()
        .or_else(|| schema["kind"].as_str())
        .or_else(|| schema["number"].as_str())
}
fn resolved<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    if let Some(name) = schema.get("ref").and_then(Value::as_str) {
        resolved(&root["definitions"][name], root)
    } else {
        schema
    }
}
fn validate(v: &Value, schema: &Value, root: &Value, path: &str) {
    let schema = resolved(schema, root);
    assert!(!schema.is_null(), "missing schema: {path}");
    match v {
        Value::Number(n) => {
            let kind = scalar_kind(schema).expect("valid vector schema and fixture");
            assert!(
                matches!(kind, "integer" | "float"),
                "unclassified number: {path}"
            );
            if kind == "integer" {
                assert!(
                    n.as_f64().expect("valid vector schema and fixture").fract() == 0.,
                    "noninteger: {path}"
                );
            }
        }
        Value::Array(xs) => {
            for (i, x) in xs.iter().enumerate() {
                validate(x, &schema["items"], root, &format!("{path}[{i}]"));
            }
        }
        Value::Object(xs) => {
            for (key, x) in xs {
                let child = schema["fields"]
                    .get(key)
                    .or_else(|| {
                        if key.parse::<usize>().is_ok() {
                            schema.get("indexed")
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| panic!("missing schema: {path}.{key}"));
                validate(x, child, root, &format!("{path}.{key}"));
            }
        }
        Value::String(s) => {
            let kind = scalar_kind(schema);
            // Same rule as the TS comparator: strings are exact data, or the three encoded
            // non-finite sentinels at a float node. Nothing else may sit at a numeric node.
            assert!(
                kind == Some("exact")
                    || (kind == Some("float")
                        && matches!(s.as_str(), "NaN" | "Infinity" | "-Infinity")),
                "string at a node declared {}, only non-finite sentinels are allowed there: {path}",
                kind.unwrap_or("structured")
            );
        }
        Value::Bool(_) => assert!(
            scalar_kind(schema) == Some("exact"),
            "boolean at a node not declared exact: {path}"
        ),
        Value::Null => assert!(
            schema["nullable"] == true,
            "null at a node not declared nullable: {path}"
        ),
    }
}
fn compare(a: &Value, b: &Value, schema: &Value, root: &Value, path: &str) {
    let schema = resolved(schema, root);
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            let kind = scalar_kind(schema).expect("valid vector schema and fixture");
            if kind == "integer" {
                assert_eq!(a, b, "{path}");
            } else {
                let (a, b) = (
                    a.as_f64().expect("valid vector schema and fixture"),
                    b.as_f64().expect("valid vector schema and fixture"),
                );
                assert!(
                    (a - b).abs() <= (1e-12 * a.abs().max(b.abs())).max(1e-15),
                    "{path}: {a} != {b}"
                );
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &schema["items"], root, &format!("{path}[{i}]"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(
                a.keys().collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{path}"
            );
            for (key, a) in a {
                compare(
                    a,
                    &b[key],
                    schema["fields"].get(key).unwrap_or(&schema["indexed"]),
                    root,
                    &format!("{path}.{key}"),
                );
            }
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}
/// Validate both complete documents before comparing any values.
pub fn assert_vectors(area: &str, actual: &Value, expected: &Value) {
    let root = load("schema");
    let schema = &root["areas"][area];
    validate(actual, schema, &root, area);
    validate(expected, schema, &root, area);
    compare(actual, expected, schema, &root, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_comparison_accepts_whole_valued_float_tolerance() {
        let expected = load("filters");
        let mut actual = expected.clone();
        actual["cases"][0]["output"][0] = serde_json::json!(1.000_000_000_000_1);
        assert_vectors("filters", &actual, &expected);
    }
    #[test]
    #[should_panic(expected = "filters.cases[0].output[0]")]
    fn rejects_float_outside_tolerance() {
        let expected = load("filters");
        let mut actual = expected.clone();
        actual["cases"][0]["output"][0] = serde_json::json!(1.000_000_01);
        assert_vectors("filters", &actual, &expected);
    }
    #[test]
    #[should_panic(expected = "schemaVersion")]
    fn rejects_integer_drift() {
        let expected = load("filters");
        let mut actual = expected.clone();
        actual["schemaVersion"] = serde_json::json!(2);
        assert_vectors("filters", &actual, &expected);
    }
    #[test]
    #[should_panic(expected = "missing schema")]
    fn checks_coverage_even_for_equal_documents() {
        let mut unknown = load("filters");
        unknown["cases"][0]["input"]["unknown"] = serde_json::json!(1.);
        assert_vectors("filters", &unknown, &unknown);
    }
    #[test]
    #[should_panic(expected = "missing schema")]
    fn validates_expected_before_comparing() {
        let actual = load("filters");
        let mut expected = actual.clone();
        expected["schemaVersion"] = serde_json::json!(2);
        expected["cases"][0]["input"]["unknown"] = serde_json::json!(1.);
        assert_vectors("filters", &actual, &expected);
    }
    #[test]
    #[should_panic(expected = "boolean at a node not declared exact")]
    fn rejects_boolean_at_a_float_node() {
        let mut document = load("filters");
        document["cases"][0]["output"][0] = serde_json::json!(true);
        assert_vectors("filters", &document, &document);
    }
    #[test]
    #[should_panic(expected = "only non-finite sentinels are allowed")]
    fn rejects_arbitrary_string_at_a_float_node() {
        let mut document = load("filters");
        document["cases"][0]["output"][0] = serde_json::json!("abc");
        assert_vectors("filters", &document, &document);
    }
    #[test]
    fn accepts_encoded_nonfinite_strings_at_a_float_node() {
        let mut document = load("filters");
        document["cases"][0]["output"][0] = serde_json::json!("-Infinity");
        assert_vectors("filters", &document, &document);
    }
    #[test]
    #[should_panic(expected = "only non-finite sentinels are allowed")]
    fn rejects_nonfinite_string_at_an_integer_node() {
        let mut document = load("filters");
        document["schemaVersion"] = serde_json::json!("NaN");
        assert_vectors("filters", &document, &document);
    }
    #[test]
    #[should_panic(expected = "not declared nullable")]
    fn rejects_null_at_a_non_nullable_node() {
        let mut document = load("filters");
        document["cases"][0]["name"] = serde_json::Value::Null;
        assert_vectors("filters", &document, &document);
    }
}
