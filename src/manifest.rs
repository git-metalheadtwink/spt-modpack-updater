use anyhow::{Context, Result};
use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub description: String,
    pub spt_version: String,
    pub author: Option<String>,
    pub website: Option<String>,
    pub changelog: Vec<String>,
}

pub fn read_manifest(path: &Path) -> Result<Manifest> {
    let file_path = path.join("manifest.json");

    let content = fs::read_to_string(&file_path)
        .with_context(|| format!("Failed to read manifest at {:?}", file_path))?;

    let manifest: Manifest = serde_json::from_str(&content)
        .with_context(|| "Invalid manifest.json format")?;

    Ok(manifest)
}