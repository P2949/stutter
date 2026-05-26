use std::path::Path;

use anyhow::Result;
use serde::de::DeserializeOwned;

use super::{
    load_json::load_json_file,
    loader::ArtifactLoader,
    paths::{artifact_input_path, push_unique_string},
};
use crate::{
    artifacts::{ArtifactEncoding, ArtifactKind, artifact_file_name, artifact_path, artifact_spec},
    recorder::{MetadataFile, SessionFile},
};

impl<'a> ArtifactLoader<'a> {
    pub fn load_required_json<T: DeserializeOwned>(&mut self, kind: ArtifactKind) -> Result<T> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::JsonObject {
            anyhow::bail!("artifact {:?} is not a JSON object", kind);
        }

        let path = artifact_path(self.run_dir, kind);
        if !path.exists() {
            anyhow::bail!(
                "missing mandatory {} (searched {})",
                artifact_file_name(kind),
                path.display()
            );
        }

        push_unique_string(&mut self.validation.present_files, artifact_file_name(kind));
        load_json_file(&path)
    }
}

pub fn load_session(path: &Path) -> Result<SessionFile> {
    let session_path = artifact_input_path(path, ArtifactKind::Session);
    load_json_file(&session_path)
}

pub fn load_metadata(path: &Path) -> Result<Option<MetadataFile>> {
    let metadata_path = artifact_input_path(path, ArtifactKind::Metadata);
    if !metadata_path.exists() {
        return Ok(None);
    }
    load_json_file(&metadata_path).map(Some)
}
