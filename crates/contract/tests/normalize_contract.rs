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
