use anyhow::Result;
use serde::de::DeserializeOwned;

use super::{
    load_json::load_ndjson_file_filtered,
    loader::ArtifactLoader,
    paths::{file_name_for_path, push_unique_string},
};
use crate::artifacts::{
    ArtifactEncoding, ArtifactKind, artifact_file_name, artifact_path,
    artifact_primary_and_alias_paths, artifact_spec,
};

impl<'a> ArtifactLoader<'a> {
    pub fn load_optional_json<T: DeserializeOwned>(
        &mut self,
        kind: ArtifactKind,
    ) -> Result<Option<T>> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::JsonObject {
            anyhow::bail!("artifact {:?} is not a JSON object", kind);
        }

        let path = artifact_path(self.run_dir, kind);
        if !path.exists() {
            push_unique_string(
                &mut self.validation.missing_optional_files,
                artifact_file_name(kind),
            );
            return Ok(None);
        }

        push_unique_string(&mut self.validation.present_files, artifact_file_name(kind));
        super::load_json::load_json_file(&path).map(Some)
    }

    pub fn load_optional_ndjson<T: DeserializeOwned>(
        &mut self,
        kind: ArtifactKind,
    ) -> Result<Vec<T>> {
        self.load_optional_ndjson_filtered(kind, |_| true)
    }

    pub fn load_optional_ndjson_with_aliases<T: DeserializeOwned>(
        &mut self,
        kind: ArtifactKind,
    ) -> Result<Vec<T>> {
        self.load_optional_ndjson_filtered_with_aliases(kind, |_| true)
    }

    pub fn load_optional_ndjson_filtered<T: DeserializeOwned, F: Fn(&T) -> bool>(
        &mut self,
        kind: ArtifactKind,
        filter: F,
    ) -> Result<Vec<T>> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::Ndjson {
            anyhow::bail!("artifact {:?} is not an NDJSON stream", kind);
        }

        let file_name = artifact_file_name(kind);
        let path = artifact_path(self.run_dir, kind);
        if !path.exists() {
            push_unique_string(&mut self.validation.missing_optional_files, file_name);
            return Ok(Vec::new());
        }

        push_unique_string(&mut self.validation.present_files, file_name);
        load_ndjson_file_filtered(&path, filter)
    }

    pub fn load_optional_ndjson_filtered_with_aliases<T: DeserializeOwned, F: Fn(&T) -> bool>(
        &mut self,
        kind: ArtifactKind,
        filter: F,
    ) -> Result<Vec<T>> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::Ndjson {
            anyhow::bail!("artifact {:?} is not an NDJSON stream", kind);
        }

        for path in artifact_primary_and_alias_paths(self.run_dir, kind) {
            if path.exists() {
                let file_name = file_name_for_path(&path);
                push_unique_string(&mut self.validation.present_files, file_name);
                self.validation
                    .missing_optional_files
                    .retain(|missing| missing != artifact_file_name(kind));
                return load_ndjson_file_filtered(&path, filter);
            }
        }

        push_unique_string(
            &mut self.validation.missing_optional_files,
            artifact_file_name(kind),
        );
        Ok(Vec::new())
    }
}
