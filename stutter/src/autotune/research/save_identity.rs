//! The identity of the game save a long session starts from. Two long runs are comparable only when they begin at the
//! same place in the game, so the authorized work unit's definition covers the save as well as the route: a digest over
//! the save directory's file names, sizes and contents. Read-only; nothing here opens or loads the game.

use std::{io::Read, path::Path};

use super::{RouteStep, definition_sha256_of};

/// A save has a handful of small files; anything larger or deeper is not one.
const MAX_FILES: usize = 64;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// SHA-256 (hex) over one save directory: every regular file directly inside it, by name, size and content, in name
/// order. A symlink, a nested directory, an unreadable or oversized file, or an empty directory is no save at all.
pub fn save_identity_sha256(dir: &Path) -> Result<String, &'static str> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|_| "SaveUnreadable")? {
        let entry = entry.map_err(|_| "SaveUnreadable")?;
        let kind = entry.file_type().map_err(|_| "SaveUnreadable")?;
        if !kind.is_file() {
            return Err("SaveHasNonRegularEntry");
        }
        names.push(
            entry
                .file_name()
                .into_string()
                .map_err(|_| "SaveNameInvalid")?,
        );
    }
    if names.is_empty() || names.len() > MAX_FILES {
        return Err("SaveFileCountInvalid");
    }
    names.sort();
    let mut text = String::from("stutter-save-v1\n");
    for name in names {
        let path = dir.join(&name);
        let file = std::fs::File::open(&path).map_err(|_| "SaveUnreadable")?;
        let length = file.metadata().map_err(|_| "SaveUnreadable")?.len();
        if length > MAX_FILE_BYTES {
            return Err("SaveFileTooLarge");
        }
        let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
        file.take(MAX_FILE_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|_| "SaveUnreadable")?;
        text.push_str(&format!(
            "{name}:{}:{}\n",
            bytes.len(),
            stutter_core::hash::sha256_hex(&bytes)
        ));
    }
    Ok(stutter_core::hash::sha256_hex(text.as_bytes()))
}

/// The definition digest of a whole long session: the route, its script and the save it starts from.
pub fn session_definition_sha256(
    route: &[RouteStep],
    script_sha256: &str,
    save_sha256: &str,
) -> String {
    let route_digest = definition_sha256_of(route, script_sha256);
    stutter_core::hash::sha256_hex(
        format!("stutter-long-session-v1\nroute:{route_digest}\nsave:{save_sha256}\n").as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, bytes) in files {
            std::fs::write(dir.path().join(name), bytes).unwrap();
        }
        dir
    }

    #[test]
    fn a_save_is_identified_by_every_file_and_a_changed_byte_or_name_is_another_save() {
        let a = save(&[("sav.dat", b"world"), ("metadata.9.json", b"{}")]);
        let same = save(&[("metadata.9.json", b"{}"), ("sav.dat", b"world")]);
        let id = save_identity_sha256(a.path()).unwrap();
        assert_eq!(
            id,
            save_identity_sha256(same.path()).unwrap(),
            "order is by name"
        );
        let edited = save(&[("sav.dat", b"worle"), ("metadata.9.json", b"{}")]);
        let renamed = save(&[("sav.bak", b"world"), ("metadata.9.json", b"{}")]);
        let extra = save(&[
            ("sav.dat", b"world"),
            ("metadata.9.json", b"{}"),
            ("x", b""),
        ]);
        for other in [&edited, &renamed, &extra] {
            assert_ne!(id, save_identity_sha256(other.path()).unwrap());
        }
    }

    #[test]
    fn a_directory_that_is_not_a_plain_save_has_no_identity() {
        let empty = save(&[]);
        assert_eq!(
            save_identity_sha256(empty.path()),
            Err("SaveFileCountInvalid")
        );
        let nested = save(&[("sav.dat", b"w")]);
        std::fs::create_dir(nested.path().join("sub")).unwrap();
        assert_eq!(
            save_identity_sha256(nested.path()),
            Err("SaveHasNonRegularEntry")
        );
        let linked = save(&[("sav.dat", b"w")]);
        std::os::unix::fs::symlink("/etc/hostname", linked.path().join("link")).unwrap();
        assert_eq!(
            save_identity_sha256(linked.path()),
            Err("SaveHasNonRegularEntry")
        );
        assert_eq!(
            save_identity_sha256(Path::new("/nonexistent/save")),
            Err("SaveUnreadable")
        );
    }

    #[test]
    fn the_session_definition_changes_with_the_route_the_script_or_the_save() {
        let route = vec![RouteStep {
            phase: "travel".into(),
            seconds: 60,
        }];
        let base = session_definition_sha256(&route, &"a".repeat(64), &"b".repeat(64));
        let longer = vec![RouteStep {
            phase: "travel".into(),
            seconds: 61,
        }];
        assert_ne!(
            base,
            session_definition_sha256(&longer, &"a".repeat(64), &"b".repeat(64))
        );
        assert_ne!(
            base,
            session_definition_sha256(&route, &"c".repeat(64), &"b".repeat(64))
        );
        assert_ne!(
            base,
            session_definition_sha256(&route, &"a".repeat(64), &"c".repeat(64))
        );
        assert_eq!(
            base,
            session_definition_sha256(&route, &"a".repeat(64), &"b".repeat(64))
        );
    }
}
