use anyhow::{Context, Result};
use serde_json::Value;
use std::{fs, path::Path};

pub fn detect_spt_version(game_path: &Path) -> Result<String> {
    let path = game_path.join("SPT/SPT.Server.runtimeconfig.json");

    let content = fs::read_to_string(&path)
        .with_context(|| format!("Missing SPT runtime file at {:?}", path))?;

    let json: Value = serde_json::from_str(&content)?;

    let key = json["targets"]
        .as_object()
        .and_then(|t| t.keys().next())
        .ok_or_else(|| anyhow::anyhow!("Invalid SPT runtime format"))?;

    // Example:
    // SPT.Server/4.0.13-RELEASE+hash

    let version = key
        .split('/')
        .nth(1)
        .unwrap_or("unknown");

    let clean = version
        .split('-')
        .next()
        .unwrap_or(version);

    Ok(clean.to_string())
}