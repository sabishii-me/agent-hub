//! Emit the normalized OpenAPI document from `contract/v1.json`.
//! The single normalization authority's CLI: `contract/openapi.json` is this
//! program's output, never hand-edited.
use std::process::ExitCode;

fn main() -> ExitCode {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let src = format!("{root}/contract/v1.json");
    let raw = match std::fs::read_to_string(&src) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("read {src}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let contract: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("parse {src}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match agent_hub_contract::project_openapi(&contract) {
        Ok(doc) => {
            let mut out = serde_json::to_string_pretty(&doc).unwrap();
            out.push('\n');
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("normalize: {e}");
            ExitCode::FAILURE
        }
    }
}
