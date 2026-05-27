use std::fs;

use anyhow::Result;

use super::io::*;

pub fn list_scenarios() -> Result<()> {
    let dir = default_scenario_dir();
    if !dir.exists() {
        return Ok(());
    }

    println!("Scenarios:");
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let extension = path.extension().and_then(|s| s.to_str());
        let stem = path.file_stem().and_then(|s| s.to_str());
        if let (Some("toml"), Some(name)) = (extension, stem) {
            println!("  - {}", name);
        }
    }
    Ok(())
}
