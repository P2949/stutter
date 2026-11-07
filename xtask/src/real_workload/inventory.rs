//! What real-workload material this host actually has, and the honest disposition of it.
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::process::executable_on_path;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Inventory {
    pub controlled_tools: Vec<ToolStatus>,
    pub steam_titles: Vec<String>,
    /// Titles on the preregistered allowlist of games with a repeatable unattended benchmark or
    /// replay. Empty means no real game qualifies, whatever is installed.
    pub qualifying_titles: Vec<String>,
    pub real_game_disposition: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolStatus {
    pub name: String,
    pub path: Option<PathBuf>,
}

/// `"name"` of a Steam `appmanifest_*.acf`.
pub fn manifest_name(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split('"').filter(|part| !part.trim().is_empty());
        (parts.next()? == "name").then(|| parts.next().map(str::to_owned))?
    })
}

pub fn steam_titles(steamapps: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(steamapps) else {
        return Vec::new();
    };
    let mut titles: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("appmanifest_") && name.ends_with(".acf")
        })
        .filter_map(|entry| manifest_name(&fs::read_to_string(entry.path()).ok()?))
        .filter(|name| {
            // Runtimes and tools are not games.
            !name.starts_with("Proton")
                && !name.starts_with("Steam Linux Runtime")
                && !name.starts_with("Steamworks")
        })
        .collect();
    titles.sort();
    titles.dedup();
    titles
}

pub fn collect(steamapps: &Path, allowlist: &[String]) -> Inventory {
    let controlled_tools = ["vkcube", "mangohud", "glxgears", "gamescope", "vkmark"]
        .into_iter()
        .map(|name| ToolStatus {
            name: name.to_owned(),
            path: executable_on_path(name),
        })
        .collect();
    let titles = steam_titles(steamapps);
    let qualifying: Vec<String> = titles
        .iter()
        .filter(|title| allowlist.contains(title))
        .cloned()
        .collect();
    let real_game_disposition = if qualifying.is_empty() {
        "BlockedResource: no installed title is on the allowlist of games with a repeatable unattended benchmark or replay; no human play is requested and no synthetic run is presented as a game".to_owned()
    } else {
        "eligible: run the preregistered comparison on the qualifying titles".to_owned()
    };
    Inventory {
        controlled_tools,
        steam_titles: titles,
        qualifying_titles: qualifying,
        real_game_disposition,
    }
}
