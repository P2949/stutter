use std::{fs, path::{Path, PathBuf}};
use anyhow::{Context, bail};

pub fn run_no_allow_attrs(root: &Path) -> anyhow::Result<()> {
    let matches = find_allow_attributes(root)?;
    if matches.is_empty() {
        return Ok(());
    }

    println!("Rust allow attributes are forbidden:");
    for allow_match in matches {
        println!(
            "{}:{}: {}",
            allow_match.path.display(),
            allow_match.line,
            allow_match.text.trim()
        );
    }

    bail!("remove Rust allow attributes and fix the lint directly")
}

#[derive(Debug, Eq, PartialEq)]
pub struct AllowAttributeMatch {
    pub path: PathBuf,
    pub line: usize,
    pub text: String,
}

pub fn find_allow_attributes(root: &Path) -> anyhow::Result<Vec<AllowAttributeMatch>> {
    let mut matches = Vec::new();
    for source_dir in rust_source_roots(root) {
        collect_allow_attributes(root, &source_dir, &mut matches)?;
    }
    Ok(matches)
}

fn rust_source_roots(root: &Path) -> Vec<PathBuf> {
    [
        "stutter",
        "stutter-common",
        "stutter-config",
        "stutter-core",
        "stutter-ebpf",
        "stutter-report",
        "xtask",
    ]
    .into_iter()
    .map(|path| root.join(path))
    .collect()
}

fn collect_allow_attributes(
    root: &Path,
    dir: &Path,
    matches: &mut Vec<AllowAttributeMatch>,
) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to read file type for {}", path.display()))?;

        if file_type.is_dir() {
            collect_allow_attributes(root, &path, matches)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            scan_allow_attribute_file(root, &path, matches)?;
        }
    }

    Ok(())
}

const OUTER_ATTR_PREFIX: &str = "#[";
const INNER_ATTR_PREFIX: &str = "#![";
const ALLOW_CALL: &str = "allow(";

pub fn scan_allow_attribute_file(
    root: &Path,
    path: &Path,
    matches: &mut Vec<AllowAttributeMatch>,
) -> anyhow::Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let relative_path = path.strip_prefix(root).unwrap_or(path);

    let inner_allow = format!("{INNER_ATTR_PREFIX}{ALLOW_CALL}");
    let outer_allow = format!("{OUTER_ATTR_PREFIX}{ALLOW_CALL}");

    for (line_index, line) in content.lines().enumerate() {
        let compact = line.split_whitespace().collect::<String>();
        if compact.contains(&inner_allow)
            || compact.contains(&outer_allow)
            || (compact.contains("cfg_attr(") && compact.contains(ALLOW_CALL))
        {
            matches.push(AllowAttributeMatch {
                path: relative_path.to_path_buf(),
                line: line_index + 1,
                text: line.to_owned(),
            });
        }
    }

    Ok(())
}
