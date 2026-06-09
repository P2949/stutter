use std::path::PathBuf;

pub fn default_community_rules_dir() -> Option<PathBuf> {
    default_user_rules_dir()
}

pub fn default_user_rules_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME")
        && !xdg.trim().is_empty()
    {
        return Some(PathBuf::from(xdg).join("stutter").join("community-rules"));
    }

    std::env::var("HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(|home| {
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("stutter")
                .join("community-rules")
        })
}

pub fn default_system_rules_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/local/share/stutter/community-rules"),
        PathBuf::from("/usr/share/stutter/community-rules"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            // SAFETY: callers hold TEST_MUTEX before mutating the process
            // environment, keeping these tests serialized.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }

        fn unset(key: &'static str) -> Self {
            let old = std::env::var(key).ok();
            // SAFETY: callers hold TEST_MUTEX before mutating the process
            // environment, keeping these tests serialized.
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(old) = &self.old {
                // SAFETY: EnvGuard is used while TEST_MUTEX is held, so restore
                // mutations are serialized with the matching test body.
                unsafe {
                    std::env::set_var(self.key, old);
                }
            } else {
                // SAFETY: EnvGuard is used while TEST_MUTEX is held, so restore
                // mutations are serialized with the matching test body.
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn default_user_rules_dir_uses_xdg_data_home() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let _xdg = EnvGuard::set("XDG_DATA_HOME", "/tmp/stutter-xdg-data");
        let _home = EnvGuard::set("HOME", "/tmp/stutter-home");

        assert_eq!(
            default_user_rules_dir().unwrap(),
            PathBuf::from("/tmp/stutter-xdg-data")
                .join("stutter")
                .join("community-rules")
        );
    }

    #[test]
    fn default_user_rules_dir_falls_back_to_home() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let _xdg = EnvGuard::unset("XDG_DATA_HOME");
        let _home = EnvGuard::set("HOME", "/tmp/stutter-home");

        assert_eq!(
            default_user_rules_dir().unwrap(),
            PathBuf::from("/tmp/stutter-home")
                .join(".local")
                .join("share")
                .join("stutter")
                .join("community-rules")
        );
    }

    #[test]
    fn default_system_rules_dirs_are_share_directories() {
        assert_eq!(
            default_system_rules_dirs(),
            vec![
                PathBuf::from("/usr/local/share/stutter/community-rules"),
                PathBuf::from("/usr/share/stutter/community-rules"),
            ]
        );
    }
}
