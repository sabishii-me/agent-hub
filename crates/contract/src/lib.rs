//! The **single normalization authority** for the wire contract (`ARCHITECTURE` §9, task T1).
//!
//! `contract/v1.json` is a *compact DSL*, not JSON Schema:
//! request/response fragments are written as short type strings, e.g.
//! `"string?"`, `"string|null?"`, `"'manual'|string?"`, `"integer[]"`,
//! `"boolean|null"`, `{"#ref": "..."}`, or a nested object literal.
//!
//! Those strings must **never** be handed to a JSON-Schema validator as-is
//! (that is the reproduced G3 defect: `type: []`, `nullable: true`,
//! `type: "binary"`, `const: "manual"`, and optional fields projected as
//! `required`). This crate turns the DSL into **valid JSON Schema 2020-12**,
//! deterministically, in one place.
//!
//! # The DSL
//!
//! A *type string* is a `|`-separated union of atoms; a trailing `?` on the
//! whole string means "omittable field" (it does **not** add `null`).
//!
//! | atom        | JSON Schema                                                |
//! |-------------|------------------------------------------------------------|
//! | `string`    | `{"type":"string"}`                                        |
//! | `integer`   | `{"type":"integer"}`                                       |
//! | `number`    | `{"type":"number"}`                                        |
//! | `boolean`   | `{"type":"boolean"}`                                       |
//! | `null`      | `{"type":"null"}`                                          |
//! | `'a'`       | `{"const":"a"}`                                            |
//! | `#/defs/x`  | `{"$ref":"#/$defs/x"}`                                     |
//! | `array`     | `{"type":"array"}` (untyped items; see `elements`)         |
//! | `object`    | `{"type":"object"}`                                        |
//! | `text`      | `{"type":"string","contentMediaType":"text/plain"}`        |
//! | `file`      | `{"type":"string","contentEncoding":"base64"}`             |
//! | `secret`    | `{"type":"string","writeOnly":true}`                       |
//! | `binary`    | `{"type":"string","contentEncoding":"base64"}`             |
//! | `enum`      | `{"enum":[...]}` (values come from a sibling)              |
//!
//! `undefined`/`null` at the fragment root: `{}` (anything).

pub mod errors;

pub use errors::{ErrorEntry, ErrorTable};

use serde_json::{json, Map, Value};
use serde_json::Map as JMap;

/// The literal used for `nullable`-style unions: any `X|null` → `{"type":["X","null"]}`
/// (or `anyOf` when `X` is not a bare type name).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NormalizeError {
    #[error("empty type string at {path}")]
    Empty { path: String },
    #[error("unknown atom `{atom}` at {path}")]
    UnknownAtom { atom: String, path: String },
    #[error("`?` is not allowed on a union member: `{s}` at {path}")]
    OptionalInUnion { s: String, path: String },
}

/// Result of normalizing a fragment: the schema plus whether the *field* is
/// optional (a trailing `?`). Optionality is a property of the containing
/// object's `required` list, not of the type.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    pub schema: Value,
    pub optional: bool,
}

impl Normalized {
    pub fn required(schema: Value) -> Self {
        Normalized { schema, optional: false }
    }
}

/// Normalize a DSL fragment into JSON Schema 2020-12.
pub fn normalize_fragment(v: &Value, path: &str) -> Result<Normalized, NormalizeError> {
    match v {
        Value::Null => Ok(Normalized { schema: json!({}), optional: true }),
        Value::Bool(_) | Value::Number(_) => {
            // A literal default/example; treat as the const it is.
            Ok(Normalized::required(v.clone()))
        }
        Value::String(s) => normalize_type_string(s, path),
        Value::Array(items) => {
            let mut variants = Vec::new();
            for (i, it) in items.iter().enumerate() {
                let n = normalize_fragment(it, &format!("{path}[{i}]"))?;
                variants.push(n.schema);
            }
            Ok(Normalized::required(json!({ "anyOf": variants })))
        }
        Value::Object(map) => normalize_object(map, path),
    }
}

/// Keywords that mark an object as **already a JSON Schema**, not a field map.
const SCHEMA_KEYWORDS: &[&str] = &[
    "$ref", "#ref", "type", "enum", "const", "items", "properties",
    "oneOf", "anyOf", "allOf", "description", "contentMediaType",
    "contentEncoding", "writeOnly", "readOnly", "format",
];

fn is_schema_object(map: &Map<String, Value>) -> bool {
    map.keys().any(|k| SCHEMA_KEYWORDS.contains(&k.as_str()))
}

fn normalize_object(map: &Map<String, Value>, path: &str) -> Result<Normalized, NormalizeError> {
    // Fragment object literal: {field: <fragment>, ...}. A leading `#ref`
    // key is the reference form.
    if let Some(Value::String(target)) = map.get("#ref") {
        return Ok(Normalized::required(json!({ "$ref": ref_target(target)? })));
    }
    if let Some(Value::String(target)) = map.get("$ref") {
        return Ok(Normalized::required(json!({ "$ref": target })));
    }
    // The value is itself a schema (`{"type":"array","items":{...}}`,
    // `{"type":"null","description":"..."}`) - pass through, rewriting refs.
    if is_schema_object(map) {
        return Ok(Normalized::required(normalize_schema(
            &Value::Object(map.clone()),
            path,
        )?));
    }

    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, frag) in map {
        let n = normalize_fragment(frag, &format!("{path}.{name}"))?;
        properties.insert(name.clone(), n.schema);
        if !n.optional {
            required.push(Value::String(name.clone()));
        }
    }
    let mut schema = Map::new();
    schema.insert("type".into(), Value::String("object".into()));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), Value::Array(required));
    }
    Ok(Normalized::required(Value::Object(schema)))
}

/// `#/defs/x` in the source becomes `#/$defs/x` in JSON Schema.
fn ref_target(target: &str) -> Result<String, NormalizeError> {
    if let Some(rest) = target.strip_prefix("#/defs/") {
        Ok(format!("#/$defs/{rest}"))
    } else {
        Ok(target.to_string())
    }
}

fn normalize_type_string(s: &str, path: &str) -> Result<Normalized, NormalizeError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(NormalizeError::Empty { path: path.into() });
    }
    let (body, optional) = match s.strip_suffix('?') {
        Some(b) => (b, true),
        None => (s, false),
    };
    if body.is_empty() {
        // A bare `?` field: omittable, unconstrained.
        return Ok(Normalized { schema: json!({}), optional: true });
    }
    if body.contains('|') {
        // A trailing `?` applies to the whole field, not to the last member:
        // `'manual'|string?` is an optional field of type `'manual'|string`.
        let mut variants = Vec::new();
        for atom in body.split('|') {
            let atom = atom.trim();
            if atom.is_empty() {
                return Err(NormalizeError::Empty { path: path.into() });
            }
            variants.push(atom_schema(atom, path)?);
        }
        return Ok(Normalized { schema: union(variants), optional });
    }
    Ok(Normalized { schema: atom_schema(body, path)?, optional })
}

fn union(mut variants: Vec<Value>) -> Value {
    // Merge bare {type:...} variants into a single `type` array, like the
    // canonical JSON Schema form for a nullable scalar.
    let mut types: Vec<Value> = Vec::new();
    let mut others: Vec<Value> = Vec::new();
    for v in variants.drain(..) {
        if let Value::Object(m) = &v {
            if m.len() == 1 {
                if let Some(Value::String(t)) = m.get("type") {
                    types.push(Value::String(t.clone()));
                    continue;
                }
            }
        }
        others.push(v);
    }
    if others.is_empty() {
        return json!({ "type": types });
    }
    if types.len() == 1 {
        let mut m = Map::new();
        m.insert("type".into(), types.remove(0));
        others.insert(0, Value::Object(m));
    } else if !types.is_empty() {
        others.insert(0, json!({ "type": types }));
    }
    json!({ "anyOf": others })
}

fn atom_schema(atom: &str, path: &str) -> Result<Value, NormalizeError> {
    // `X[]` is an array of X (e.g. `string[]`, `integer[]`).
    if let Some(inner) = atom.strip_suffix("[]") {
        return Ok(json!({ "type": "array", "items": atom_schema(inner, path)? }));
    }
    if let Some(quoted) = atom.strip_prefix('\'') {
        let inner = quoted
            .strip_suffix('\'')
            .ok_or_else(|| NormalizeError::UnknownAtom { atom: atom.into(), path: path.into() })?;
        return Ok(json!({ "const": inner }));
    }
    if atom.starts_with("#/") {
        return Ok(json!({ "$ref": ref_target(atom)? }));
    }
    let schema = match atom {
        "string" => json!({ "type": "string" }),
        "integer" => json!({ "type": "integer" }),
        "number" => json!({ "type": "number" }),
        "boolean" => json!({ "type": "boolean" }),
        "null" => json!({ "type": "null" }),
        "array" => json!({ "type": "array" }),
        "object" => json!({ "type": "object" }),
        "text" => json!({ "type": "string", "contentMediaType": "text/plain" }),
        "file" => json!({ "type": "string", "contentEncoding": "base64" }),
        "binary" => json!({ "type": "string", "contentEncoding": "base64" }),
        "secret" => json!({ "type": "string", "writeOnly": true }),
        _ => return Err(NormalizeError::UnknownAtom { atom: atom.into(), path: path.into() }),
    };
    Ok(schema)
}


/// Project the whole `contract/v1.json` into an OpenAPI 3.1 document whose
/// schemas are **valid JSON Schema 2020-12**.
///
/// This is the single normalization authority (T1): the old generator emitted
/// `type: []`, `nullable: true`, `type: "binary"`, `const: "manual"` and
/// marked optional fields required. This projection cannot express those.
pub fn project_openapi(contract: &Value) -> Result<Value, NormalizeError> {
    use serde_json::Map as JMap;

    let title = contract.get("title").and_then(Value::as_str).unwrap_or("agent-hub");
    let version = contract
        .get("version")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "\"0\"".into());

    // defs -> $defs, keys unchanged, nested #/defs rewritten.
    let mut defs = JMap::new();
    if let Some(src_defs) = contract.get("defs").and_then(Value::as_object) {
        for (name, def) in src_defs {
            let path = format!("#/$defs/{name}");
            // A def is either a field-map (like an endpoint body) or a schema.
            let normalized = match def {
                Value::Object(m) if !is_schema_object(m) => {
                    normalize_object(m, &path)?.schema
                }
                _ => normalize_schema(def, &path)?,
            };
            defs.insert(name.clone(), normalized);
        }
    }

    let mut paths = JMap::new();
    if let Some(endpoints) = contract.get("endpoints").and_then(Value::as_array) {
        for ep in endpoints {
            let method = ep.get("method").and_then(Value::as_str).unwrap_or("get");
            let path = ep.get("path").and_then(Value::as_str).unwrap_or("/");
            let op = build_operation(ep, path)?;

            let entry = paths
                .entry(path.to_string())
                .or_insert_with(|| Value::Object(JMap::new()));
            if let Value::Object(m) = entry {
                m.insert(method.to_ascii_lowercase(), op);
            }
        }
    }

    let mut components = JMap::new();
    components.insert("schemas".into(), Value::Object(defs));

    Ok(json!({
        "openapi": "3.1.0",
        "info": { "title": title, "version": version },
        "paths": Value::Object(paths),
        "components": Value::Object(components),
    }))
}

fn build_operation(ep: &Value, path: &str) -> Result<Value, NormalizeError> {
    use serde_json::Map as JMap;
    let mut op = JMap::new();
    if let Some(d) = ep.get("description").and_then(Value::as_str) {
        op.insert("description".into(), Value::String(d.into()));
    }

    let mut params = Vec::new();
    if let Some(path_params) = extract_path_params(path) {
        for name in path_params {
            params.push(json!({
                "name": name,
                "in": "path",
                "required": true,
                "schema": { "type": "string" }
            }));
        }
    }

    // request body
    if let Some(req) = ep.get("request") {
        if req.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            let n = normalize_object(req.as_object().unwrap(), &format!("{path}.request"))?;
            if !params.is_empty() {
                op.insert("parameters".into(), Value::Array(params.clone()));
            }
            op.insert(
                "requestBody".into(),
                json!({
                    "required": true,
                    "content": { "application/json": { "schema": n.schema } }
                }),
            );
        }
    }

    // response
    if let Some(resp) = ep.get("response") {
        let schema = if resp.is_null() {
            json!({})
        } else if let Some(o) = resp.as_object() {
            normalize_object(o, &format!("{path}.response"))?.schema
        } else {
            normalize_fragment(resp, &format!("{path}.response"))?.schema
        };
        // A long command answers `202 Accepted` + `Location` (ADR-0009, §13.2);
        // the body names the resource and the Location header points at it.
        let accepted = ep.get("accepted").and_then(Value::as_bool).unwrap_or(false);
        let code = if accepted { "202" } else { "200" };
        let desc = if accepted { "accepted" } else { "success" };
        let mut responses = Map::new();
        responses.insert(
            code.into(),
            json!({
                "description": desc,
                "content": { "application/json": { "schema": schema } }
            }),
        );
        if accepted {
            // The Location header is part of the 202 response.
            responses[code]["headers"] = json!({
                "Location": { "schema": { "type": "string" }, "description": "the resource URL" }
            });
        }
        op.insert("responses".into(), Value::Object(responses));
    } else if !params.is_empty() {
        op.insert("parameters".into(), Value::Array(params));
    }

    Ok(Value::Object(op))
}

fn extract_path_params(path: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let end = after.find('}')?;
        out.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}


/// Normalize a **schema object** (from `defs` or a field value) into strict
/// JSON Schema 2020-12, recursively:
///
/// - `"nullable": true` -> the null type joins the node's type
///   (`{"type":"string","nullable":true}` -> `{"type":["string","null"]}`).
/// - `"type": "binary"` -> `{"type":"string","contentEncoding":"base64"}`.
/// - a string DSL fragment -> the atom schema (`"string|null"`, `"integer[]"`).
/// - `#/defs/x` refs -> `#/$defs/x`.
///
/// `description` and other keywords are preserved.
pub fn normalize_schema(v: &Value, path: &str) -> Result<Value, NormalizeError> {
    match v {
        Value::String(_) => Ok(normalize_fragment(v, path)?.schema),
        Value::Object(map) => {
            let mut out = JMap::new();
            for (k, val) in map {
                match k.as_str() {
                    "nullable" => {} // handled below, dropped as a keyword
                    "$ref" | "#ref" => {
                        if let Value::String(t) = val {
                            out.insert("$ref".into(), Value::String(ref_target(t)?));
                        }
                    }
                    // A map of schemas: recurse per entry (each value is a
                    // schema), never treat the map itself as one schema.
                    "properties" | "$defs" | "definitions" | "dependentSchemas" => {
                        if let Value::Object(m) = val {
                            let mut norm = JMap::new();
                            for (name, sub) in m {
                                norm.insert(
                                    name.clone(),
                                    normalize_schema(sub, &format!("{path}.{k}.{name}"))?,
                                );
                            }
                            out.insert(k.clone(), Value::Object(norm));
                        } else {
                            out.insert(k.clone(), val.clone());
                        }
                    }
                    // A schema or array of schemas: recurse.
                    "items" | "additionalProperties" | "oneOf" | "anyOf" | "allOf"
                    | "not" | "contains" | "propertyNames" | "if" | "then" | "else"
                    | "prefixItems" => {
                        out.insert(k.clone(), normalize_schema(val, &format!("{path}.{k}"))?);
                    }
                    _ => {
                        out.insert(k.clone(), val.clone());
                    }
                }
            }

            // Fold `nullable: true` into the node's type.
            if map.get("nullable").and_then(Value::as_bool) == Some(true) {
                add_null_type(&mut out);
            }

            // `type: "binary"` is not a JSON Schema type.
            if out.get("type").and_then(Value::as_str) == Some("binary") {
                out.insert("type".into(), Value::String("string".into()));
                out.insert("contentEncoding".into(), Value::String("base64".into()));
            }

            Ok(Value::Object(out))
        }
        Value::Array(a) => {
            let mut out = Vec::with_capacity(a.len());
            for (i, x) in a.iter().enumerate() {
                out.push(normalize_schema(x, &format!("{path}[{i}]"))?);
            }
            Ok(Value::Array(out))
        }
        other => Ok(other.clone()),
    }
}

/// Join `null` into a node's `type`.
fn add_null_type(out: &mut JMap<String, Value>) {
    match out.get("type") {
        Some(Value::String(t)) => {
            if t != "null" {
                out.insert(
                    "type".into(),
                    json!([t.clone(), "null"]),
                );
            }
        }
        Some(Value::Array(a)) => {
            let mut a = a.clone();
            if !a.iter().any(|x| x == "null") {
                a.push(Value::String("null".into()));
                out.insert("type".into(), Value::Array(a));
            }
        }
        Some(_) => {
            // A non-type-shaped `type` (e.g. an array already); leave it.
        }
        None => {
            // nullable without a type: any JSON value or null -> leave as {}.
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn norm(s: &str) -> Value {
        normalize_fragment(&Value::String(s.into()), "t").unwrap().schema
    }

    #[test]
    fn null_union_is_type_array() {
        assert_eq!(norm("string|null"), json!({ "type": ["string", "null"] }));
    }

    #[test]
    fn optional_does_not_add_null() {
        let n = normalize_fragment(&Value::String("string?".into()), "t").unwrap();
        assert!(n.optional);
        assert_eq!(n.schema, json!({ "type": "string" }));
    }

    #[test]
    fn const_union_not_const_manual() {
        let v = norm("'manual'|string");
        assert_eq!(v, json!({ "anyOf": [ { "type": "string" }, { "const": "manual" } ] }));
    }

    #[test]
    fn binary_is_base64_string() {
        assert_eq!(norm("binary"), json!({ "type": "string", "contentEncoding": "base64" }));
    }

    #[test]
    fn ref_maps_defs() {
        assert_eq!(norm("#/defs/session"), json!({ "$ref": "#/$defs/session" }));
    }

    #[test]
    fn fragment_object_respects_optionality() {
        let frag = json!({ "a": "string", "b": "string?" });
        let n = normalize_fragment(&frag, "t").unwrap();
        let req = n.schema.get("required").unwrap().as_array().unwrap();
        assert_eq!(req, &vec![Value::String("a".into())]);
        assert!(!req.contains(&Value::String("b".into())));
    }
}
