use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;

const CATEGORIES: &[&str] = &[
    "dma_fence",
    "dma_buf",
    "sync_file",
    "drm_sched",
    "amdgpu",
    "i915",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DrmFenceTracepointField {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub signed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DrmFenceTracepointFormat {
    pub category: String,
    pub name: String,
    pub fields: Vec<DrmFenceTracepointField>,
}

impl DrmFenceTracepointFormat {
    pub fn find_field(&self, names: &[&str]) -> Option<&DrmFenceTracepointField> {
        self.fields
            .iter()
            .find(|field| names.iter().any(|name| field.name == *name))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DrmFenceTracepointCategory {
    pub category: String,
    pub status: String,
    pub tracepoints: Vec<DrmFenceTracepointFormat>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DrmFenceTracepointDiscovery {
    pub events_root: PathBuf,
    pub categories: Vec<DrmFenceTracepointCategory>,
    pub supported_profile: String,
}

impl DrmFenceTracepointDiscovery {
    #[cfg(test)]
    pub fn category(&self, name: &str) -> Option<&DrmFenceTracepointCategory> {
        self.categories
            .iter()
            .find(|category| category.category == name)
    }

    pub fn selected_wait_start(&self) -> Option<&DrmFenceTracepointFormat> {
        self.find_tracepoint(|name| {
            name.contains("wait") && (name.contains("start") || name.contains("begin"))
        })
    }

    pub fn selected_wait_done(&self) -> Option<&DrmFenceTracepointFormat> {
        self.find_tracepoint(|name| {
            name.contains("wait")
                && (name.contains("done") || name.contains("end") || name.contains("finish"))
        })
    }

    pub fn selected_signal(&self) -> Option<&DrmFenceTracepointFormat> {
        self.find_tracepoint(|name| {
            name.contains("signal")
                || name.contains("signaled")
                || (name.contains("job")
                    && (name.contains("done")
                        || name.contains("end")
                        || name.contains("finish")
                        || name.contains("complete")))
        })
    }

    fn find_tracepoint(
        &self,
        predicate: impl Fn(&str) -> bool,
    ) -> Option<&DrmFenceTracepointFormat> {
        self.categories
            .iter()
            .flat_map(|category| category.tracepoints.iter())
            .find(|tracepoint| predicate(&tracepoint.name.to_ascii_lowercase()))
    }
}

pub fn discover_drm_fence_tracepoints_default() -> DrmFenceTracepointDiscovery {
    for events_root in [
        Path::new("/sys/kernel/tracing/events"),
        Path::new("/sys/kernel/debug/tracing/events"),
    ] {
        if events_root.exists() {
            return discover_drm_fence_tracepoints(events_root);
        }
    }

    discover_drm_fence_tracepoints(Path::new("/sys/kernel/tracing/events"))
}

pub fn discover_drm_fence_tracepoints(events_root: &Path) -> DrmFenceTracepointDiscovery {
    let categories = CATEGORIES
        .iter()
        .map(|category| discover_category(events_root, category))
        .collect::<Vec<_>>();
    let supported_profile = supported_profile(&categories);

    DrmFenceTracepointDiscovery {
        events_root: events_root.to_path_buf(),
        categories,
        supported_profile,
    }
}

pub fn render_text(discovery: &DrmFenceTracepointDiscovery) -> String {
    let mut out = String::new();
    out.push_str("DRM fence tracepoint discovery:\n");
    for category in &discovery.categories {
        out.push_str(&format!("  {}: {}\n", category.category, category.status));
        for tracepoint in category.tracepoints.iter().take(8) {
            out.push_str(&format!(
                "    - {} fields={}\n",
                tracepoint.name,
                format_field_names(&tracepoint.fields)
            ));
        }
        for warning in &category.warnings {
            out.push_str(&format!("    warning: {warning}\n"));
        }
    }
    out.push_str(&format!(
        "  supported profile: {}\n",
        discovery.supported_profile
    ));
    out
}

fn discover_category(events_root: &Path, category: &str) -> DrmFenceTracepointCategory {
    let root = events_root.join(category);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return DrmFenceTracepointCategory {
                category: category.to_owned(),
                status: "unavailable".to_owned(),
                tracepoints: Vec::new(),
                warnings: Vec::new(),
            };
        }
        Err(err) => {
            return DrmFenceTracepointCategory {
                category: category.to_owned(),
                status: "unreadable".to_owned(),
                tracepoints: Vec::new(),
                warnings: vec![err.to_string()],
            };
        }
    };

    let mut tracepoints = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_fence_related_tracepoint(category, &name) {
            continue;
        }

        let path = root.join(&name).join("format");
        match fs::read_to_string(&path) {
            Ok(contents) => tracepoints.push(parse_tracepoint_format(category, &name, &contents)),
            Err(err) => warnings.push(format!("{}: {}", path.display(), err)),
        }
    }
    tracepoints.sort_by(|a, b| a.name.cmp(&b.name));

    DrmFenceTracepointCategory {
        category: category.to_owned(),
        status: if tracepoints.is_empty() {
            "unavailable".to_owned()
        } else {
            "available".to_owned()
        },
        tracepoints,
        warnings,
    }
}

pub fn parse_tracepoint_format(
    category: impl Into<String>,
    name: impl Into<String>,
    contents: &str,
) -> DrmFenceTracepointFormat {
    DrmFenceTracepointFormat {
        category: category.into(),
        name: name.into(),
        fields: contents.lines().filter_map(parse_field_line).collect(),
    }
}

fn parse_field_line(line: &str) -> Option<DrmFenceTracepointField> {
    let mut name = None;
    let mut offset = None;
    let mut size = None;
    let mut signed = None;

    for part in line.split(';') {
        let part = part.trim();
        if let Some(declaration) = part.strip_prefix("field:") {
            name = parse_field_name(declaration);
        } else if let Some(value) = part.strip_prefix("offset:") {
            offset = value.trim().parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("size:") {
            size = value.trim().parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("signed:") {
            signed = Some(value.trim() != "0");
        }
    }

    Some(DrmFenceTracepointField {
        name: name?,
        offset: offset?,
        size: size?,
        signed: signed.unwrap_or(false),
    })
}

fn parse_field_name(declaration: &str) -> Option<String> {
    let token = declaration.split_whitespace().last()?;
    let token = token.trim_start_matches('*');
    let token = token.split('[').next().unwrap_or(token).trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn supported_profile(categories: &[DrmFenceTracepointCategory]) -> String {
    let available = |name: &str| {
        categories
            .iter()
            .any(|category| category.category == name && category.status == "available")
    };

    if available("dma_fence") {
        "generic dma_fence".to_owned()
    } else if available("amdgpu") && available("i915") {
        "amdgpu+i915 partial".to_owned()
    } else if available("drm_sched") && available("amdgpu") {
        "amdgpu render partial".to_owned()
    } else if available("i915") {
        "i915 display partial".to_owned()
    } else if available("drm_sched") {
        "drm_sched partial".to_owned()
    } else {
        "unavailable".to_owned()
    }
}

fn is_fence_related_tracepoint(category: &str, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match category {
        "dma_fence" | "dma_buf" | "sync_file" => {
            contains_any(&lower, &["fence", "wait", "signal", "sync", "dma_buf"])
        }
        "drm_sched" => contains_any(&lower, &["job", "sched", "fence", "run", "done"]),
        "amdgpu" | "i915" => contains_any(
            &lower,
            &["fence", "wait", "signal", "sched", "job", "request"],
        ),
        _ => false,
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn format_field_names(fields: &[DrmFenceTracepointField]) -> String {
    if fields.is_empty() {
        "-".to_owned()
    } else {
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_format(events_root: &Path, category: &str, name: &str, contents: &str) {
        let dir = events_root.join(category).join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("format"), contents).unwrap();
    }

    #[test]
    fn discovery_reports_amdgpu_i915_partial_profile() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write_format(
            root,
            "amdgpu",
            "amdgpu_job_run",
            "field:u64 context;\toffset:8;\tsize:8;\tsigned:0;\n",
        );
        write_format(
            root,
            "i915",
            "i915_request_wait_begin",
            "field:u64 seqno;\toffset:16;\tsize:8;\tsigned:0;\n",
        );

        let discovery = discover_drm_fence_tracepoints(root);

        assert_eq!(discovery.supported_profile, "amdgpu+i915 partial");
        assert_eq!(discovery.category("amdgpu").unwrap().status, "available");
        assert!(render_text(&discovery).contains("supported profile: amdgpu+i915 partial"));
    }

    #[test]
    fn parser_extracts_field_layout() {
        let format = parse_tracepoint_format(
            "dma_fence",
            "dma_fence_wait_start",
            "field:u64 context;\toffset:8;\tsize:8;\tsigned:0;\n\
             field:int ret;\toffset:16;\tsize:4;\tsigned:1;\n",
        );

        assert_eq!(format.fields[0].name, "context");
        assert_eq!(format.fields[1].name, "ret");
        assert!(format.fields[1].signed);
    }
}
