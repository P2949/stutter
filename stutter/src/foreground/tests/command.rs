//! Foreground helper command-resolution and environment hardening tests.
//!
//! Owns tests for trusted helper lookup and sanitized helper subprocess execution. Does not own
//! compositor-specific parser behavior.

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    use super::super::{
        super::command::{resolve_trusted_foreground_helper, trusted_foreground_command},
        restore_env_var,
    };

    fn write_executable(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn trusted_helper_resolution_ignores_malicious_path_entry() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_path = std::env::var_os("PATH");
        let root = crate::test_support::TestRoot::new("foreground-helper-malicious-path");
        let malicious_swaymsg = root.join("swaymsg");
        write_executable(&malicious_swaymsg, "#!/bin/sh\nexit 99\n");

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("PATH", root.path().as_os_str());
        }

        let resolved = resolve_trusted_foreground_helper("swaymsg");

        assert_ne!(resolved.as_deref(), Some(malicious_swaymsg.as_path()));
        // SAFETY: TEST_MUTEX is still held and previous_path was captured before mutation.
        unsafe {
            restore_env_var("PATH", previous_path);
        }
    }

    #[test]
    fn trusted_helper_resolution_accepts_configured_absolute_path() {
        let root = crate::test_support::TestRoot::new("foreground-helper-absolute-path");
        let helper = root.join("custom-hyprctl");
        write_executable(&helper, "#!/bin/sh\nexit 0\n");

        assert_eq!(
            resolve_trusted_foreground_helper(helper.to_str().unwrap()).as_deref(),
            Some(helper.as_path())
        );
    }

    #[test]
    fn trusted_foreground_command_uses_minimal_sanitized_environment() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_path = std::env::var_os("PATH");
        let previous_ld_preload = std::env::var_os("LD_PRELOAD");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let root = crate::test_support::TestRoot::new("foreground-helper-sanitized-env");
        let helper = root.join("print-env");
        write_executable(
            &helper,
            r#"#!/bin/sh
printf 'PATH=%s\n' "$PATH"
printf 'SWAYSOCK=%s\n' "$SWAYSOCK"
printf 'LD_PRELOAD=%s\n' "$LD_PRELOAD"
            "#,
        );

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("PATH", root.path().as_os_str());
            std::env::set_var("LD_PRELOAD", "/tmp/stutter-unsafe-preload.so");
            std::env::set_var("SWAYSOCK", "/tmp/stutter-test-sway.sock");
        }

        let output = trusted_foreground_command(&helper).output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();

        assert!(output.status.success());
        assert!(stdout.contains("PATH=/usr/bin:/bin"));
        assert!(stdout.contains("SWAYSOCK=/tmp/stutter-test-sway.sock"));
        assert!(stdout.contains("LD_PRELOAD="));
        assert!(!stdout.contains("/tmp/stutter-unsafe-preload.so"));

        // SAFETY: TEST_MUTEX is still held and previous values were captured before mutation.
        unsafe {
            restore_env_var("PATH", previous_path);
            restore_env_var("LD_PRELOAD", previous_ld_preload);
            restore_env_var("SWAYSOCK", previous_swaysock);
        }
    }
}
