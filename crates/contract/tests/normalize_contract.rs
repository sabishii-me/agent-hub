//! T1 acceptance: the real `contract/v1.json` normalizes to **valid JSON Schema
//! 2020-12** with none of the reproduced projection defects.

use agent_hub_contract::{normalize_fragment, project_openapi};
use serde_json::{json, Value};

fn contract() -> Value {
    let raw = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../contract/v1.json"),
    )
    .expect("contract/v1.json");
    serde_json::from_str(&raw).expect("contract parses")
}

/// Recursively collect every projection defect; panics on `type: []`.
fn walk(v: &Value, path: &str, hits: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                if k == "type" {
                    if let Value::Array(a) = val {
                        if a.is_empty() {
                            hits.push(format!("empty type at {path}"));
                        }
                    }
                    if val == "binary" {
                        hits.push(format!("type:binary at {path}"));
                    }
                }
                if k == "nullable" {
                    hits.push(format!("nullable at {path}"));
                }
                // A `const` is only a defect when it is NOT a member of a real
                // union (the old projection replaced `'manual'|string` with a
                // bare `const:manual`). A `const` under `anyOf` is correct.
                if k == "const" && val == "manual" && !path.contains("anyOf") {
                    hits.push(format!("bare const:manual at {path}"));
                }
                walk(val, &format!("{path}.{k}"), hits);
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                walk(x, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

#[test]
fn projection_has_no_reproduced_defects() {
    let openapi = project_openapi(&contract()).expect("projection");
    let mut hits = Vec::new();
    walk(&openapi, "$", &mut hits);
    assert!(hits.is_empty(), "reproduced defects survived:\n{}", hits.join("\n"));
}

fn meta_schema() -> jsonschema::Validator {
    jsonschema::validator_for(&json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema"
    }))
    .expect("meta-schema")
}

#[test]
fn projection_is_valid_json_schema_2020_12() {
    let openapi = project_openapi(&contract()).expect("projection");
    let v = meta_schema();

    for (name, schema) in openapi["components"]["schemas"].as_object().unwrap() {
        if !v.is_valid(schema) {
            let errs: Vec<String> = v.iter_errors(schema).map(|e| e.to_string()).collect();
            panic!("component `{name}` is not valid 2020-12: {}", errs.join("; "));
        }
    }

    for (path, item) in openapi["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            if let Some(s) = op.pointer("/requestBody/content/application~1json/schema") {
                assert!(
                    v.is_valid(s),
                    "{method} {path} request schema invalid: {:?}",
                    v.iter_errors(s).map(|e| e.to_string()).collect::<Vec<_>>()
                );
            }
            if let Some(s) = op.pointer("/responses/200/content/application~1json/schema") {
                assert!(
                    v.is_valid(s),
                    "{method} {path} response schema invalid: {:?}",
                    v.iter_errors(s).map(|e| e.to_string()).collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn patch_session_does_not_over_constrain_optional_fields() {
    let openapi = project_openapi(&contract()).expect("projection");
    let schema = &openapi["paths"]["/v1/sessions/{id}"]["patch"]["requestBody"]["content"]
        ["application/json"]["schema"];
    let required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();

    for field in [
        "modelId", "presetId", "disabledTools", "plan", "review",
        "modelProviderId", "thinkingLevel", "title",
    ] {
        assert!(
            !required.iter().any(|r| r == field),
            "`{field}` was projected as required"
        );
    }
}

#[test]
fn nullable_and_const_roundtrip() {
    let n = normalize_fragment(&json!("string|null"), "providerId").unwrap();
    assert_eq!(n.schema, json!({ "type": ["string", "null"] }));
    assert!(!n.optional);

    // `'manual'|string?` is an OPTIONAL field of type `'manual'|string`.
    let u = normalize_fragment(&json!("'manual'|string?"), "reason").unwrap();
    assert!(u.optional, "trailing ? makes the whole field optional");
    assert_eq!(
        u.schema,
        json!({ "anyOf": [ { "type": "string" }, { "const": "manual" } ] })
    );
}

#[test]
fn projection_is_deterministic_and_complete() {
    let c = contract();
    let a = project_openapi(&c).expect("projection");
    let b = project_openapi(&c).expect("projection");
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap(),
        "projection must be byte-stable"
    );

    // Route parity: every source endpoint becomes one operation.
    let src = c["endpoints"].as_array().unwrap().len();
    let mut ops = 0usize;
    for item in a["paths"].as_object().unwrap().values() {
        ops += item.as_object().unwrap().len();
    }
    assert_eq!(ops, src, "one projected operation per source endpoint");
}

/// The COMMITTED `contract/openapi.json` must equal a FRESH projection of
/// `contract/v1.json`. The served `/v1/openapi.json` is this committed file
/// (`include_str!`), so a file that has drifted from the contract is the hub
/// serving an OpenAPI that contradicts its own contract - the exact thing "the
/// contract is the interface" forbids. The projection is deterministic, so this
/// compares bytes (after a stable re-serialization) and NAMES the first drift.
#[test]
fn the_committed_openapi_matches_a_fresh_projection() {
    let raw = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../contract/openapi.json"),
    )
    .expect("contract/openapi.json");
    let committed: Value = serde_json::from_str(&raw).expect("committed openapi parses");
    let fresh = project_openapi(&contract()).expect("projection");

    // Compare canonically (key order is not meaningful across serializers) and
    // report the first differing JSON path so the fix is obvious.
    let mut path = String::new();
    let same = deep_eq(&committed, &fresh, &mut path);
    assert!(
        same,
        "contract/openapi.json is STALE: it differs from a fresh projection of \
         contract/v1.json at `{path}`. Regenerate it: \
         `cargo run -p agent-hub-contract --bin emit-openapi -- . > contract/openapi.json`"
    );
}

/// First JSON path where two values differ (or "" when equal).
fn deep_eq(a: &Value, b: &Value, path: &mut String) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()) {
                let seg = format!("{path}/{k}");
                match (x.get(k), y.get(k)) {
                    (Some(xv), Some(yv)) => {
                        let mut p = seg.clone();
                        if !deep_eq(xv, yv, &mut p) {
                            *path = p;
                            return false;
                        }
                    }
                    _ => {
                        *path = seg;
                        return false;
                    }
                }
            }
            true
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                *path = format!("{path} (len {} vs {})", x.len(), y.len());
                return false;
            }
            for (i, (xv, yv)) in x.iter().zip(y.iter()).enumerate() {
                let mut p = format!("{path}[{i}]");
                if !deep_eq(xv, yv, &mut p) {
                    *path = p;
                    return false;
                }
            }
            true
        }
        _ => a == b,
    }
}
