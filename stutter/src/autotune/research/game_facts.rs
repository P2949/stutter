//! Conservative procfs `GameFacts` provider. It names a process the game only when several
//! independent facts agree: the Cyberpunk process name, the intent owner, the Steam app id in
//! its environment, and the installed game executable as a command-line argument. A
//! Cyberpunk-named process that fails any of those is an error, never "no game", so an
//! unattributable process can neither be mistaken for the game nor for its absence.
//!
//! How Wine and Proton actually present the game in procfs (the argument spelling, the process
//! name) is not yet qualified against a real launch: where this provider cannot attribute the
//! process it fails closed, which the observation source reads as an identity mismatch.

use std::{
    fs,
    path::{Path, PathBuf},
};

use stutter_core::ids::Pid;

use super::{
    ProtonSource,
    cyberpunk::CyberpunkProcessIdentity,
    native_source::{GameFacts, Sighting},
};
use crate::telemetry::proc_view::live_process;

const MAX_PROC_ENTRIES: usize = 65_536;
const MAX_FILE: u64 = 256 * 1024;
const GAME_COMM_PREFIX: &str = "Cyberpunk2077";
/// The first command-line word (the program) of a Windows process, case-insensitively.
fn windows_program(cmdline: &[u8]) -> Option<String> {
    let first = cmdline.split(|byte| *byte == 0).next()?;
    Some(String::from_utf8_lossy(first).into_owned())
}

/// Digest of the arm's own settings file.
type SettingsDigest = Box<dyn FnMut(ProtonSource) -> anyhow::Result<String> + Send>;
type Clock = Box<dyn FnMut() -> anyhow::Result<u128> + Send>;

pub struct ProcfsGameFacts {
    proc_root: PathBuf,
    uid: u32,
    app_id: u32,
    build_id: u64,
    /// The installed game executable, re-hashed once and matched against the process command line.
    executable: PathBuf,
    executable_sha256: Option<String>,
    arm: Option<ProtonSource>,
    settings: SettingsDigest,
    clock: Clock,
}

impl ProcfsGameFacts {
    pub fn new(
        proc_root: PathBuf,
        uid: u32,
        app_id: u32,
        build_id: u64,
        executable: PathBuf,
        settings: SettingsDigest,
        clock: Clock,
    ) -> Self {
        Self {
            proc_root,
            uid,
            app_id,
            build_id,
            executable,
            executable_sha256: None,
            arm: None,
            settings,
            clock,
        }
    }

    /// The Windows-style paths Wine can report for the installed executable: the `Z:` root mapping
    /// and, for a game inside a Steam library, Proton's `S:` mapping of that library's `steamapps`.
    fn wine_paths(&self) -> anyhow::Result<Vec<String>> {
        let path = self
            .executable
            .to_str()
            .filter(|path| path.starts_with('/'))
            .ok_or_else(|| anyhow::anyhow!("game executable path is not absolute UTF-8"))?;
        let mut forms = vec![format!("Z:{}", path.replace('/', "\\"))];
        if let Some(index) = path.rfind("/steamapps/") {
            forms.push(format!("S:{}", path[index..].replace('/', "\\")));
        }
        Ok(forms)
    }

    fn executable_digest(&mut self) -> anyhow::Result<String> {
        if let Some(digest) = &self.executable_sha256 {
            return Ok(digest.clone());
        }
        let metadata = fs::metadata(&self.executable)?;
        anyhow::ensure!(metadata.is_file(), "game executable is not a regular file");
        let digest = stutter_core::hash::sha256_hex(&fs::read(&self.executable)?);
        self.executable_sha256 = Some(digest.clone());
        Ok(digest)
    }
}

fn read_small(path: &Path) -> anyhow::Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    anyhow::ensure!(metadata.len() <= MAX_FILE, "procfs record is too large");
    let bytes = fs::read(path)?;
    anyhow::ensure!(bytes.len() as u64 <= MAX_FILE, "procfs record is too large");
    Ok(bytes)
}

fn nul_fields(bytes: &[u8]) -> Vec<&[u8]> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect()
}

impl GameFacts for ProcfsGameFacts {
    fn sight(&mut self) -> anyhow::Result<Sighting> {
        let expected_paths = self.wine_paths()?;
        let expected_env = format!("SteamAppId={}", self.app_id);
        let entries = fs::read_dir(&self.proc_root)?;
        let mut found = Vec::new();
        let mut scanned = 0usize;
        for entry in entries {
            let name = entry?.file_name();
            let Some(pid) = name.to_str().and_then(|name| name.parse::<u32>().ok()) else {
                continue;
            };
            scanned += 1;
            anyhow::ensure!(scanned <= MAX_PROC_ENTRIES, "too many processes to scan");
            let dir = self.proc_root.join(pid.to_string());
            // A process that vanishes mid-scan was not the game we are attributing.
            let Ok(comm) = read_small(&dir.join("comm")) else {
                continue;
            };
            let Ok(cmdline) = read_small(&dir.join("cmdline")) else {
                continue;
            };
            // The game renames its main thread (its comm is not the executable name), so the
            // program path in the command line is the identity; the comm is only an extra trigger.
            let runs_installed_game = windows_program(&cmdline).is_some_and(|program| {
                expected_paths
                    .iter()
                    .any(|path| path.eq_ignore_ascii_case(&program))
            });
            if !runs_installed_game && !String::from_utf8_lossy(&comm).starts_with(GAME_COMM_PREFIX)
            {
                continue;
            }
            // From here the process is named like the game: any failed attribution is an error.
            let live = live_process(&self.proc_root, pid)
                .ok_or_else(|| anyhow::anyhow!("a game-named process could not be read"))?;
            anyhow::ensure!(
                live.uid == self.uid,
                "a game-named process has another owner"
            );
            let environ = read_small(&dir.join("environ"))?;
            anyhow::ensure!(
                nul_fields(&environ).contains(&expected_env.as_bytes()),
                "a game-named process is not this Steam app"
            );
            anyhow::ensure!(
                runs_installed_game
                    || nul_fields(&cmdline).iter().any(|field| {
                        expected_paths
                            .iter()
                            .any(|path| path.as_bytes().eq_ignore_ascii_case(field))
                    }),
                "a game-named process does not run the installed executable"
            );
            found.push((pid, live.start_ticks));
        }
        match found.as_slice() {
            [] => Ok(Sighting::None),
            [(pid, start_ticks)] => Ok(Sighting::One(CyberpunkProcessIdentity {
                pid: Pid::new(*pid),
                start_ticks: *start_ticks,
                uid: self.uid,
                app_id: self.app_id,
                build_id: self.build_id,
                executable_sha256: self.executable_digest()?,
            })),
            _ => Ok(Sighting::Multiple),
        }
    }

    fn begin_arm(&mut self, arm: ProtonSource) {
        self.arm = Some(arm);
    }

    fn loaded_runtime(
        &mut self,
        pid: u32,
    ) -> anyhow::Result<Option<super::loaded_runtime::LoadedRuntime>> {
        use std::io::Read;
        const MAPS_LIMIT: u64 = 16 * 1024 * 1024;
        let file = fs::File::open(self.proc_root.join(pid.to_string()).join("maps"))?;
        let mut bytes = Vec::new();
        file.take(MAPS_LIMIT + 1).read_to_end(&mut bytes)?;
        // A capture cut at the limit would hide mappings; it is an error, not a smaller summary.
        anyhow::ensure!(
            bytes.len() as u64 <= MAPS_LIMIT,
            "the process memory map is larger than the capture limit"
        );
        Ok(Some(super::loaded_runtime::summarize(&bytes)?))
    }

    fn settings_sha256(&mut self) -> anyhow::Result<String> {
        let arm = self
            .arm
            .ok_or_else(|| anyhow::anyhow!("no Proton arm is selected for the settings read"))?;
        (self.settings)(arm)
    }

    fn now_unix_nanos(&mut self) -> anyhow::Result<u128> {
        (self.clock)()
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    const APP: u32 = 1_091_500;

    struct Fixture {
        proc: tempfile::TempDir,
        exe: tempfile::NamedTempFile,
    }

    impl Fixture {
        fn new() -> Self {
            let exe = tempfile::NamedTempFile::new().unwrap();
            fs::write(exe.path(), b"game-image").unwrap();
            Self {
                proc: tempfile::tempdir().unwrap(),
                exe,
            }
        }

        fn wine_arg(&self) -> String {
            format!("Z:{}", self.exe.path().to_str().unwrap().replace('/', "\\"))
        }

        fn add(&self, pid: u32, comm: &str, env: &[&str], args: &[&str], start: u64) {
            let dir = self.proc.path().join(pid.to_string());
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
            fs::write(
                dir.join("stat"),
                format!("{pid} ({comm}) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {start} 0 0"),
            )
            .unwrap();
            let join = |items: &[&str]| {
                items
                    .iter()
                    .flat_map(|item| item.bytes().chain(std::iter::once(0)))
                    .collect::<Vec<u8>>()
            };
            fs::write(dir.join("environ"), join(env)).unwrap();
            fs::write(dir.join("cmdline"), join(args)).unwrap();
        }

        fn facts(&self, uid: u32) -> ProcfsGameFacts {
            ProcfsGameFacts::new(
                self.proc.path().to_path_buf(),
                uid,
                APP,
                7,
                self.exe.path().to_path_buf(),
                Box::new(|_| Ok("c".repeat(64))),
                Box::new(|| Ok(42)),
            )
        }
    }

    fn owner(fixture: &Fixture) -> u32 {
        fs::metadata(fixture.proc.path()).unwrap().uid()
    }

    #[test]
    fn exactly_one_attributable_process_is_the_game() {
        let f = Fixture::new();
        let arg = f.wine_arg();
        f.add(10, "zsh", &[], &["zsh"], 1);
        f.add(
            500,
            "Cyberpunk2077.e",
            &["SteamAppId=1091500"],
            &[&arg],
            900,
        );
        let Sighting::One(identity) = f.facts(owner(&f)).sight().unwrap() else {
            panic!("expected one game process")
        };
        assert_eq!(identity.pid, Pid::new(500));
        assert_eq!(identity.start_ticks, 900);
        assert_eq!(identity.app_id, APP);
        assert_eq!(identity.build_id, 7);
        assert_eq!(
            identity.executable_sha256,
            stutter_core::hash::sha256_hex(b"game-image")
        );
    }

    #[test]
    fn no_game_named_process_is_none_and_unrelated_processes_are_ignored() {
        let f = Fixture::new();
        f.add(10, "zsh", &[], &["zsh"], 1);
        f.add(11, "steamwebhelper", &[], &["x"], 2);
        assert_eq!(f.facts(owner(&f)).sight().unwrap(), Sighting::None);
    }

    #[test]
    fn two_attributable_processes_are_ambiguous() {
        let f = Fixture::new();
        let arg = f.wine_arg();
        for (pid, start) in [(500, 1), (501, 2)] {
            f.add(
                pid,
                "Cyberpunk2077.e",
                &["SteamAppId=1091500"],
                &[&arg],
                start,
            );
        }
        assert_eq!(f.facts(owner(&f)).sight().unwrap(), Sighting::Multiple);
    }

    #[test]
    fn a_game_named_process_that_cannot_be_attributed_is_an_error_never_absence() {
        let f = Fixture::new();
        let arg = f.wine_arg();
        // Wrong Steam app, wrong executable, and missing records each fail closed.
        f.add(500, "Cyberpunk2077.e", &["SteamAppId=570940"], &[&arg], 1);
        assert!(f.facts(owner(&f)).sight().is_err());
        let g = Fixture::new();
        g.add(
            500,
            "Cyberpunk2077.e",
            &["SteamAppId=1091500"],
            &["Z:\\elsewhere\\Cyberpunk2077.exe"],
            1,
        );
        assert!(g.facts(owner(&g)).sight().is_err());
        let h = Fixture::new();
        h.add(
            500,
            "Cyberpunk2077.e",
            &["SteamAppId=1091500"],
            &[&h.wine_arg()],
            1,
        );
        fs::remove_file(h.proc.path().join("500/environ")).unwrap();
        assert!(h.facts(owner(&h)).sight().is_err());
        // Another owner is never the intent owner's game.
        let k = Fixture::new();
        k.add(
            500,
            "Cyberpunk2077.e",
            &["SteamAppId=1091500"],
            &[&k.wine_arg()],
            1,
        );
        assert!(k.facts(owner(&k) + 1).sight().is_err());
    }

    #[test]
    fn settings_and_clock_come_from_the_injected_sources() {
        let f = Fixture::new();
        let mut facts = f.facts(owner(&f));
        // Settings belong to an arm's own prefix, so no arm means no read.
        assert!(facts.settings_sha256().is_err());
        facts.begin_arm(ProtonSource::Ge);
        assert_eq!(facts.settings_sha256().unwrap(), "c".repeat(64));
        assert_eq!(facts.now_unix_nanos().unwrap(), 42);
    }
}

#[cfg(test)]
mod real_topology_tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    /// The observed Proton process: the game renames its main thread `GameThread`, and its command
    /// line names the executable through the `S:` drive that maps the library's `steamapps`.
    #[test]
    fn the_game_is_found_by_its_s_drive_program_path_not_its_renamed_comm() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp
            .path()
            .join("steamapps/common/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"game-image").unwrap();
        let proc = tempfile::tempdir().unwrap();
        let add = |pid: u32, comm: &str, env: &[&str], args: &[&str]| {
            let dir = proc.path().join(pid.to_string());
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
            fs::write(
                dir.join("stat"),
                format!("{pid} ({comm}) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 77 0 0"),
            )
            .unwrap();
            let join = |items: &[&str]| {
                items
                    .iter()
                    .flat_map(|item| item.bytes().chain(std::iter::once(0)))
                    .collect::<Vec<u8>>()
            };
            fs::write(dir.join("environ"), join(env)).unwrap();
            fs::write(dir.join("cmdline"), join(args)).unwrap();
        };
        let s_path = "S:\\steamapps\\common\\Cyberpunk 2077\\bin\\x64\\Cyberpunk2077.exe";
        add(
            900,
            "GameThread",
            &["SteamAppId=1091500"],
            &[s_path, "-benchmark"],
        );
        // The Wine host wrapper and the Proton script only mention the launcher, never run the game.
        add(
            901,
            "steam.exe",
            &["SteamAppId=1091500"],
            &[
                "c:\\windows\\system32\\steam.exe",
                "REDprelauncher.exe",
                "-benchmark",
            ],
        );
        add(
            902,
            "python3",
            &["SteamAppId=1091500"],
            &["python3", "proton", "waitforexitandrun"],
        );
        let uid = fs::metadata(proc.path()).unwrap().uid();
        let mut facts = ProcfsGameFacts::new(
            proc.path().to_path_buf(),
            uid,
            1_091_500,
            7,
            exe,
            Box::new(|_| Ok("c".repeat(64))),
            Box::new(|| Ok(1)),
        );
        let Sighting::One(identity) = facts.sight().unwrap() else {
            panic!("the renamed game process must be sighted")
        };
        assert_eq!(identity.pid, Pid::new(900));
        // The same process under another app id is an error, never absence.
        fs::write(proc.path().join("900/environ"), b"SteamAppId=1\0").unwrap();
        assert!(facts.sight().is_err());
    }
}
