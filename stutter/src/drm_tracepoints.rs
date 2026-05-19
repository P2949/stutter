use std::{collections::BTreeMap, fs, io, path::Path};

use anyhow::Context;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrmTracepointField {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub signed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrmTracepointFormat {
    pub category: String,
    pub name: String,
    pub fields: Vec<DrmTracepointField>,
}

impl DrmTracepointFormat {
    pub fn find_field(&self, names: &[&str]) -> Option<&DrmTracepointField> {
        self.fields
            .iter()
            .find(|field| names.iter().any(|name| field.name == *name))
    }

    pub fn ref_name(&self) -> String {
        format!("{}/{}", self.category, self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KmsTracepointProvider {
    GenericDrm,
    I915,
    Amdgpu,
    Mixed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KmsTracepointAvailability {
    pub pageflip_request: Option<DrmTracepointFormat>,
    pub pageflip_done: Option<DrmTracepointFormat>,
    pub vblank_event: Option<DrmTracepointFormat>,
    pub atomic_commit: Option<DrmTracepointFormat>,
    pub provider: KmsTracepointProvider,
    pub generic_drm: Vec<DrmTracepointFormat>,
    pub i915: Vec<DrmTracepointFormat>,
    pub amdgpu: Vec<DrmTracepointFormat>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KmsTracepointKind {
    PageflipRequest,
    PageflipDone,
    VblankEvent,
    AtomicCommit,
}

impl KmsTracepointAvailability {
    pub fn unavailable() -> Self {
        empty_availability(Vec::new(), Vec::new(), Vec::new())
    }

    pub fn selected_formats(&self) -> Vec<&DrmTracepointFormat> {
        [
            self.pageflip_request.as_ref(),
            self.pageflip_done.as_ref(),
            self.vblank_event.as_ref(),
            self.atomic_commit.as_ref(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    pub fn selected_provider_name(&self) -> &'static str {
        match self.provider {
            KmsTracepointProvider::GenericDrm => "generic_drm",
            KmsTracepointProvider::I915 => "i915",
            KmsTracepointProvider::Amdgpu => "amdgpu",
            KmsTracepointProvider::Mixed => "mixed",
            KmsTracepointProvider::Unavailable => "unavailable",
        }
    }

    pub fn has_selected_tracepoints(&self) -> bool {
        !matches!(self.provider, KmsTracepointProvider::Unavailable)
            && !self.selected_formats().is_empty()
    }

    pub fn has_usable_crtc_id(&self) -> bool {
        selected_formats_have_any_field(self, &["crtc_id", "crtc", "crtc_index", "pipe"])
    }

    pub fn has_usable_sequence(&self) -> bool {
        selected_formats_have_any_field(self, &["sequence", "seq", "vbl_count", "frame", "count"])
    }

    pub fn has_usable_timestamp(&self) -> bool {
        self.has_selected_tracepoints()
    }

    pub fn selected_i915_request_done(
        &self,
    ) -> Option<(&DrmTracepointFormat, &DrmTracepointFormat)> {
        if self.provider != KmsTracepointProvider::I915 {
            return None;
        }

        Some((
            self.pageflip_request.as_ref()?,
            self.pageflip_done.as_ref()?,
        ))
    }

    pub fn i915_has_required_fields(&self) -> bool {
        self.selected_i915_request_done()
            .is_some_and(|(request, done)| has_common_flip_identity(request, done))
    }

    pub fn selected_provider_has_required_fields(&self) -> bool {
        if self.provider == KmsTracepointProvider::I915 {
            return self.i915_has_required_fields();
        }

        if matches!(
            self.provider,
            KmsTracepointProvider::Unavailable | KmsTracepointProvider::Mixed
        ) {
            return false;
        }

        if let (Some(request), Some(done)) =
            (selected_request_format(self), self.pageflip_done.as_ref())
            && has_common_flip_identity(request, done)
        {
            return true;
        }

        self.vblank_event.as_ref().is_some_and(has_flip_identity)
    }
}

pub fn discover_kms_tracepoints_default() -> KmsTracepointAvailability {
    for events_root in [
        Path::new("/sys/kernel/tracing/events"),
        Path::new("/sys/kernel/debug/tracing/events"),
    ] {
        if events_root.exists() {
            return discover_kms_tracepoints(events_root);
        }
    }

    let mut availability = empty_availability(Vec::new(), Vec::new(), Vec::new());
    availability
        .warnings
        .push("kernel tracing events directory was not found".to_owned());
    availability
}

pub fn discover_kms_tracepoints(events_root: &Path) -> KmsTracepointAvailability {
    let mut warnings = Vec::new();
    let generic_drm = discover_category(events_root, "drm", &mut warnings);
    let i915 = discover_category(events_root, "i915", &mut warnings);
    let amdgpu = discover_category(events_root, "amdgpu", &mut warnings);
    let mut availability = empty_availability(generic_drm, i915, amdgpu);
    availability.warnings = warnings;

    if let Some(provider) =
        provider_from_formats(KmsTracepointProvider::GenericDrm, &availability.generic_drm)
    {
        apply_provider(
            &mut availability,
            provider,
            KmsTracepointProvider::GenericDrm,
        );
    } else if let Some(provider) =
        provider_from_formats(KmsTracepointProvider::I915, &availability.i915)
    {
        apply_provider(&mut availability, provider, KmsTracepointProvider::I915);
    } else if let Some(provider) =
        provider_from_formats(KmsTracepointProvider::Amdgpu, &availability.amdgpu)
    {
        apply_provider(&mut availability, provider, KmsTracepointProvider::Amdgpu);
    } else {
        let mixed = mixed_provider(&availability);
        if mixed.values().any(Option::is_some) {
            availability.pageflip_request = mixed
                .get(&KmsTracepointKind::PageflipRequest)
                .and_then(Clone::clone);
            availability.pageflip_done = mixed
                .get(&KmsTracepointKind::PageflipDone)
                .and_then(Clone::clone);
            availability.vblank_event = mixed
                .get(&KmsTracepointKind::VblankEvent)
                .and_then(Clone::clone);
            availability.atomic_commit = mixed
                .get(&KmsTracepointKind::AtomicCommit)
                .and_then(Clone::clone);
            availability.provider = KmsTracepointProvider::Mixed;
        }
    }

    availability
}

pub fn parse_drm_tracepoint_format(
    category: impl Into<String>,
    name: impl Into<String>,
    contents: &str,
) -> DrmTracepointFormat {
    let fields = contents
        .lines()
        .filter_map(parse_tracepoint_field_line)
        .collect();

    DrmTracepointFormat {
        category: category.into(),
        name: name.into(),
        fields,
    }
}

fn empty_availability(
    generic_drm: Vec<DrmTracepointFormat>,
    i915: Vec<DrmTracepointFormat>,
    amdgpu: Vec<DrmTracepointFormat>,
) -> KmsTracepointAvailability {
    KmsTracepointAvailability {
        pageflip_request: None,
        pageflip_done: None,
        vblank_event: None,
        atomic_commit: None,
        provider: KmsTracepointProvider::Unavailable,
        generic_drm,
        i915,
        amdgpu,
        warnings: Vec::new(),
    }
}

fn discover_category(
    events_root: &Path,
    category: &str,
    warnings: &mut Vec<String>,
) -> Vec<DrmTracepointFormat> {
    let category_root = events_root.join(category);
    let entries = match fs::read_dir(&category_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            warnings.push(format!(
                "{category} tracepoint directory unreadable: {}",
                err
            ));
            return Vec::new();
        }
    };

    let mut formats = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_kms_related_tracepoint(category, &name) {
            continue;
        }

        match read_tracepoint_format(events_root, category, &name) {
            Ok(format) => formats.push(format),
            Err(err) => warnings.push(format!("{category}/{name} format unreadable: {err:#}")),
        }
    }

    formats.sort_by(|a, b| a.name.cmp(&b.name));
    formats
}

fn read_tracepoint_format(
    events_root: &Path,
    category: &str,
    name: &str,
) -> anyhow::Result<DrmTracepointFormat> {
    let path = events_root.join(category).join(name).join("format");
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read tracepoint format {}", path.display()))?;
    Ok(parse_drm_tracepoint_format(category, name, &contents))
}

fn provider_from_formats(
    provider: KmsTracepointProvider,
    formats: &[DrmTracepointFormat],
) -> Option<BTreeMap<KmsTracepointKind, Option<DrmTracepointFormat>>> {
    if formats.is_empty() {
        return None;
    }

    let map = classified_formats(formats);
    let has_completion = map
        .get(&KmsTracepointKind::PageflipDone)
        .and_then(Option::as_ref)
        .is_some()
        || map
            .get(&KmsTracepointKind::VblankEvent)
            .and_then(Option::as_ref)
            .is_some();
    let has_start = map
        .get(&KmsTracepointKind::PageflipRequest)
        .and_then(Option::as_ref)
        .is_some()
        || map
            .get(&KmsTracepointKind::AtomicCommit)
            .and_then(Option::as_ref)
            .is_some();

    if provider == KmsTracepointProvider::GenericDrm {
        has_completion.then_some(map)
    } else {
        (has_completion || has_start).then_some(map)
    }
}

fn apply_provider(
    availability: &mut KmsTracepointAvailability,
    provider: BTreeMap<KmsTracepointKind, Option<DrmTracepointFormat>>,
    provider_kind: KmsTracepointProvider,
) {
    availability.pageflip_request = provider
        .get(&KmsTracepointKind::PageflipRequest)
        .and_then(Clone::clone);
    availability.pageflip_done = provider
        .get(&KmsTracepointKind::PageflipDone)
        .and_then(Clone::clone);
    availability.vblank_event = provider
        .get(&KmsTracepointKind::VblankEvent)
        .and_then(Clone::clone);
    availability.atomic_commit = provider
        .get(&KmsTracepointKind::AtomicCommit)
        .and_then(Clone::clone);
    availability.provider = provider_kind;
}

fn mixed_provider(
    availability: &KmsTracepointAvailability,
) -> BTreeMap<KmsTracepointKind, Option<DrmTracepointFormat>> {
    let all = availability
        .generic_drm
        .iter()
        .chain(availability.i915.iter())
        .chain(availability.amdgpu.iter())
        .cloned()
        .collect::<Vec<_>>();
    classified_formats(&all)
}

fn classified_formats(
    formats: &[DrmTracepointFormat],
) -> BTreeMap<KmsTracepointKind, Option<DrmTracepointFormat>> {
    [
        KmsTracepointKind::PageflipRequest,
        KmsTracepointKind::PageflipDone,
        KmsTracepointKind::VblankEvent,
        KmsTracepointKind::AtomicCommit,
    ]
    .into_iter()
    .map(|kind| {
        let selected = formats
            .iter()
            .filter(|format| tracepoint_matches_kind(format, kind))
            .min_by_key(|format| tracepoint_rank(format, kind))
            .cloned();
        (kind, selected)
    })
    .collect()
}

fn tracepoint_matches_kind(format: &DrmTracepointFormat, kind: KmsTracepointKind) -> bool {
    let name = format.name.to_ascii_lowercase();
    match kind {
        KmsTracepointKind::PageflipRequest => {
            contains_any(&name, &["page_flip", "pageflip", "flip"])
                && contains_any(&name, &["request", "queue", "submit", "begin", "start"])
        }
        KmsTracepointKind::PageflipDone => {
            contains_any(&name, &["page_flip", "pageflip", "flip"])
                && contains_any(&name, &["done", "complete", "delivered", "finish", "end"])
        }
        KmsTracepointKind::VblankEvent => name.contains("vblank"),
        KmsTracepointKind::AtomicCommit => name.contains("atomic") && name.contains("commit"),
    }
}

fn tracepoint_rank(format: &DrmTracepointFormat, kind: KmsTracepointKind) -> u8 {
    let name = format.name.to_ascii_lowercase();
    match kind {
        KmsTracepointKind::PageflipRequest => {
            if contains_any(
                &name,
                &["page_flip_request", "pageflip_request", "flip_request"],
            ) {
                0
            } else if contains_any(&name, &["page_flip_queue", "pageflip_queue", "flip_queue"]) {
                1
            } else {
                2
            }
        }
        KmsTracepointKind::PageflipDone => {
            if contains_any(&name, &["page_flip_done", "pageflip_done", "flip_done"]) {
                0
            } else if contains_any(
                &name,
                &["page_flip_complete", "pageflip_complete", "flip_complete"],
            ) {
                1
            } else {
                2
            }
        }
        KmsTracepointKind::VblankEvent => {
            if name == "drm_vblank_event" {
                0
            } else {
                1
            }
        }
        KmsTracepointKind::AtomicCommit => {
            if name.contains("tail_begin") {
                0
            } else if name.contains("begin") {
                1
            } else {
                2
            }
        }
    }
}

fn is_kms_related_tracepoint(category: &str, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match category {
        "drm" => contains_any(&lower, &["flip", "vblank", "atomic", "commit", "page"]),
        "i915" | "amdgpu" => contains_any(&lower, &["flip", "vblank", "atomic", "commit"]),
        _ => false,
    }
}

fn selected_formats_have_any_field(
    availability: &KmsTracepointAvailability,
    names: &[&str],
) -> bool {
    availability.selected_formats().into_iter().any(|format| {
        format
            .fields
            .iter()
            .any(|field| names.iter().any(|name| *name == field.name))
    })
}

fn has_common_flip_identity(request: &DrmTracepointFormat, done: &DrmTracepointFormat) -> bool {
    let request_crtc = request
        .find_field(&["crtc_id", "crtc", "crtc_index"])
        .is_some();
    let done_crtc = done
        .find_field(&["crtc_id", "crtc", "crtc_index"])
        .is_some();
    let request_pipe = request.find_field(&["pipe"]).is_some();
    let done_pipe = done.find_field(&["pipe"]).is_some();

    (request_crtc && done_crtc) || (request_pipe && done_pipe)
}

fn has_flip_identity(format: &DrmTracepointFormat) -> bool {
    format
        .find_field(&["crtc_id", "crtc", "crtc_index", "pipe"])
        .is_some()
}

pub fn selected_request_format(
    availability: &KmsTracepointAvailability,
) -> Option<&DrmTracepointFormat> {
    availability
        .pageflip_request
        .as_ref()
        .or(availability.atomic_commit.as_ref())
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn parse_tracepoint_field_line(line: &str) -> Option<DrmTracepointField> {
    let mut name = None;
    let mut offset = None;
    let mut size = None;
    let mut signed = None;

    for part in line.split(';') {
        let part = part.trim();
        if let Some(declaration) = part.strip_prefix("field:") {
            name = parse_tracepoint_field_name(declaration);
        } else if let Some(value) = part.strip_prefix("offset:") {
            offset = value.trim().parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("size:") {
            size = value.trim().parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("signed:") {
            signed = Some(value.trim() != "0");
        }
    }

    Some(DrmTracepointField {
        name: name?,
        offset: offset?,
        size: size?,
        signed: signed.unwrap_or(false),
    })
}

fn parse_tracepoint_field_name(declaration: &str) -> Option<String> {
    let token = declaration.split_whitespace().last()?;
    let token = token.trim_start_matches('*');
    let token = token.split('[').next().unwrap_or(token);
    let token = token.trim();

    if token.is_empty() {
        None
    } else {
        Some(token.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format_with_fields(extra_fields: &str) -> String {
        format!(
            "name: tracepoint\nID: 1\nformat:\n\
             field:unsigned short common_type;\toffset:0;\tsize:2;\tsigned:0;\n\
             field:unsigned char common_flags;\toffset:2;\tsize:1;\tsigned:0;\n\
             {extra_fields}"
        )
    }

    fn write_format(events_root: &Path, category: &str, name: &str, contents: &str) {
        let dir = events_root.join(category).join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("format"), contents).unwrap();
    }

    #[test]
    fn parses_tracepoint_fields() {
        let format = parse_drm_tracepoint_format(
            "drm",
            "drm_vblank_event",
            &format_with_fields(
                "field:unsigned int crtc_id;\toffset:8;\tsize:4;\tsigned:0;\n\
                 field:int sequence;\toffset:12;\tsize:4;\tsigned:1;\n",
            ),
        );

        assert_eq!(format.category, "drm");
        assert_eq!(format.name, "drm_vblank_event");
        assert!(format.fields.iter().any(|field| field.name == "crtc_id"));
        assert!(
            format
                .fields
                .iter()
                .any(|field| field.name == "sequence" && field.signed)
        );
    }

    #[test]
    fn discovers_generic_drm_vblank_provider() {
        let dir = tempfile::tempdir().unwrap();
        let events_root = dir.path();
        write_format(
            events_root,
            "drm",
            "drm_vblank_event",
            &format_with_fields(
                "field:unsigned int crtc_id;\toffset:8;\tsize:4;\tsigned:0;\n\
                 field:unsigned int sequence;\toffset:12;\tsize:4;\tsigned:0;\n",
            ),
        );

        let availability = discover_kms_tracepoints(events_root);

        assert_eq!(availability.provider, KmsTracepointProvider::GenericDrm);
        assert_eq!(
            availability
                .vblank_event
                .as_ref()
                .map(|format| format.name.as_str()),
            Some("drm_vblank_event")
        );
        assert!(availability.has_usable_crtc_id());
        assert!(availability.has_usable_sequence());
        assert!(availability.has_usable_timestamp());
    }

    #[test]
    fn falls_back_to_i915_when_generic_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let events_root = dir.path();
        write_format(
            events_root,
            "i915",
            "i915_flip_request",
            &format_with_fields("field:unsigned int pipe;\toffset:8;\tsize:4;\tsigned:0;\n"),
        );
        write_format(
            events_root,
            "i915",
            "i915_flip_complete",
            &format_with_fields(
                "field:unsigned int pipe;\toffset:8;\tsize:4;\tsigned:0;\n\
                 field:unsigned int sequence;\toffset:12;\tsize:4;\tsigned:0;\n",
            ),
        );

        let availability = discover_kms_tracepoints(events_root);

        assert_eq!(availability.provider, KmsTracepointProvider::I915);
        assert_eq!(
            availability
                .pageflip_request
                .as_ref()
                .map(|format| format.name.as_str()),
            Some("i915_flip_request")
        );
        assert_eq!(
            availability
                .pageflip_done
                .as_ref()
                .map(|format| format.name.as_str()),
            Some("i915_flip_complete")
        );
        assert!(availability.has_usable_crtc_id());
        assert!(availability.i915_has_required_fields());
    }

    #[test]
    fn reports_unavailable_when_no_supported_tracepoints_exist() {
        let dir = tempfile::tempdir().unwrap();
        let availability = discover_kms_tracepoints(dir.path());

        assert_eq!(availability.provider, KmsTracepointProvider::Unavailable);
        assert!(!availability.has_selected_tracepoints());
    }
}
