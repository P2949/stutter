#![allow(dead_code)] // Transitional eBPF split: loader façade still owns object loading.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::ebpf::EbpfLoadError;

#[derive(Clone, Debug)]
pub(crate) struct EbpfObjectLoadInput {
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct EbpfObjectLoadOutput {
    pub source: EbpfObjectSource,
    pub bytes_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EbpfObjectSource {
    Embedded,
    External(PathBuf),
}

/// Read an eBPF object from an external file path.
///
/// Returns an error if the file cannot be read or is empty.
pub(crate) fn read_prebuilt_bpf_object(path: &Path) -> Result<Vec<u8>, EbpfLoadError> {
    let bytes = fs::read(path).map_err(|source| EbpfLoadError::ReadObject {
        path: path.to_path_buf(),
        source,
    })?;

    if bytes.is_empty() {
        return Err(EbpfLoadError::EmptyObject {
            path: path.to_path_buf(),
        });
    }

    Ok(bytes)
}

/// Resolve the eBPF object bytes to load.
///
/// If `STUTTER_BPF_OBJECT` is set, reads that file at runtime. This allows
/// developers to test alternate objects without rebuilding userspace, and
/// packagers to ship a separate object file.
///
/// If the env var is not set, uses the object embedded at build time via
/// `aya::include_bytes_aligned!`.
///
/// If `STUTTER_BPF_OBJECT` is set but the file is unreadable or empty, this
/// function returns an error - it does not silently fall back to the embedded
/// object.
pub(crate) fn ebpf_object_bytes() -> anyhow::Result<Cow<'static, [u8]>> {
    if let Ok(path_str) = std::env::var("STUTTER_BPF_OBJECT") {
        let path = PathBuf::from(path_str);
        log::info!("using_prebuilt_bpf_object path={}", path.display());

        let bytes = read_prebuilt_bpf_object(&path)
            .map_err(anyhow::Error::new)
            .with_context(|| format!("STUTTER_BPF_OBJECT={}", path.display()))?;

        Ok(Cow::Owned(bytes))
    } else {
        Ok(Cow::Borrowed(aya::include_bytes_aligned!(concat!(
            env!("OUT_DIR"),
            "/stutter"
        ))))
    }
}
