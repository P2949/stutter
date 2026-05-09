use std::path::{Path, PathBuf};

use crate::profiles::Profile;

#[derive(Clone, Debug)]
pub struct LoadedAutotuneProfiles {
    pub path: PathBuf,
    pub profiles: Vec<Profile>,
}

impl LoadedAutotuneProfiles {
    pub fn profile_names(&self) -> Vec<String> {
        self.profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

pub fn load_autotune_profiles(path: &Path) -> anyhow::Result<LoadedAutotuneProfiles> {
    let profiles = crate::profiles::load_profiles(path)?;

    if profiles.is_empty() {
        anyhow::bail!(
            "profile file {} did not contain [[profile]]",
            path.display()
        );
    }

    Ok(LoadedAutotuneProfiles {
        path: path.to_path_buf(),
        profiles,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-profiles-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loads_existing_profile_file_with_existing_profile_loader() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = manifest_dir
            .parent()
            .unwrap()
            .join("examples/profiles/common-game-layouts.toml");

        let loaded = load_autotune_profiles(&path).unwrap();

        assert!(!loaded.is_empty());
        assert_eq!(loaded.path, path);
        assert!(
            loaded
                .profile_names()
                .iter()
                .any(|name| name == "baseline-online")
        );
    }

    #[test]
    fn empty_profile_file_uses_tune_style_error() {
        let dir = temp_dir("empty");
        let path = dir.join("profiles.toml");
        fs::write(&path, "").unwrap();

        let err = load_autotune_profiles(&path).unwrap_err();

        assert!(err.to_string().contains("did not contain [[profile]]"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn invalid_profile_file_reuses_existing_validation() {
        let dir = temp_dir("invalid");
        let path = dir.join("profiles.toml");
        fs::write(
            &path,
            r#"
            [[profile]]
            name = "bad"

            [[profile.rules]]
            affinity = "all"
            match_class = ["Game"]
            "#,
        )
        .unwrap();

        let err = load_autotune_profiles(&path).unwrap_err();

        assert!(format!("{err:#}").contains("invalid CPU id"));

        fs::remove_dir_all(dir).ok();
    }
}
