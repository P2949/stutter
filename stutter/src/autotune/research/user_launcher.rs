//! Fixed user-manager request for the Steam launch helper. Encoding this request grants no
//! authority: the caller must first authenticate the user manager, establish an idle window,
//! persist the launch transaction, and handle a lost acknowledgement without replay.
//!
//! This only starts Steam's client command. It does not own, stop, or attest a detached game tree.
use std::{
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

use super::{AttemptSlot, ResearchIntentV1};
use crate::integrations::dbus::Writer;

pub const START_SIGNATURE: &str = "ssa(sv)a(sa(sv))";

#[derive(Debug, PartialEq, Eq)]
pub struct UserLaunchRequest {
    pub unit_name: String,
    pub body: Vec<u8>,
}

/// A fixed system Steam entry point, never a command selected through campaign IPC.
pub struct VerifiedSteamExecutable(PathBuf);

impl VerifiedSteamExecutable {
    pub fn inspect() -> anyhow::Result<Self> {
        let executable = std::fs::canonicalize("/run/current-system/sw/bin/steam")?;
        let meta = std::fs::metadata(&executable)?;
        anyhow::ensure!(
            executable.starts_with("/nix/store")
                && meta.is_file()
                && meta.uid() == 0
                && meta.mode() & 0o222 == 0
                && meta.mode() & 0o111 != 0,
            "Steam entry point is not immutable root-owned executable code"
        );
        text_path(&executable)?;
        Ok(Self(executable))
    }
}

/// Only paths read from the frozen, re-attested Steam client in the active graphical session.
pub struct VerifiedSessionEnvironment {
    uid: u32,
    home: PathBuf,
    runtime: PathBuf,
    verified_at: std::time::Instant,
}

impl VerifiedSessionEnvironment {
    pub fn inspect(
        intent: &ResearchIntentV1,
        steamapps: &Path,
        manifest: &Path,
    ) -> anyhow::Result<Self> {
        intent.validate()?;
        let uid = crate::syscall::geteuid();
        anyhow::ensure!(uid != 0 && uid == intent.uid, "wrong session owner");
        anyhow::ensure!(
            crate::telemetry::identity::read_boot_id()? == intent.boot_id
                && super::host::active_wayland_session(uid)? == intent.session_id,
            "graphical session changed"
        );
        let libraries = super::discover_steam_libraries(steamapps, uid)?;
        let inventory = super::inspect_proton_inventory(manifest, steamapps, &libraries, uid)?;
        anyhow::ensure!(
            inventory.steam_client.as_ref() == Some(&intent.steam_client)
                && intent
                    .tools
                    .iter()
                    .all(|tool| inventory.eligible_path(tool).is_some()),
            "Steam or Proton identity changed"
        );
        let client = intent
            .steam_client
            .process
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Steam process is unavailable"))?;
        let paths = super::session_env::read_session_paths(Path::new("/proc"), client, uid)?;
        let verified_at = std::time::Instant::now();
        anyhow::ensure!(
            crate::telemetry::identity::read_boot_id()? == intent.boot_id
                && super::host::active_wayland_session(uid)? == intent.session_id,
            "graphical session changed"
        );
        Ok(Self {
            uid,
            home: paths.home,
            runtime: paths.runtime,
            verified_at,
        })
    }
}

fn normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

fn text_path(path: &Path) -> anyhow::Result<&str> {
    anyhow::ensure!(
        normal_absolute(path),
        "launch path is not absolute and normalized"
    );
    let text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("launch path is not UTF-8"))?;
    anyhow::ensure!(
        text.len() <= 4096 && !text.bytes().any(|b| b == 0 || b.is_ascii_control()),
        "invalid launch path"
    );
    Ok(text)
}

// D-Bus array lengths exclude the padding before the first element. Struct elements are
// eight-byte aligned, including empty arrays (see the existing bounded wire reader).
fn array(writer: &mut Writer, alignment: usize, append: impl FnOnce(&mut Writer)) {
    writer.align(4);
    let length_at = writer.buf.len();
    writer.u32(0);
    writer.align(alignment);
    let start = writer.buf.len();
    append(writer);
    let length = (writer.buf.len() - start) as u32;
    writer.buf[length_at..length_at + 4].copy_from_slice(&length.to_le_bytes());
}

fn property(writer: &mut Writer, name: &str, signature: &str, append: impl FnOnce(&mut Writer)) {
    writer.align(8);
    writer.string(name);
    writer.signature(signature);
    append(writer);
}

/// Re-attest the fixed Steam entry point and current user session before preparing a request.
/// The returned bytes do not authorize or perform a D-Bus call.
pub fn prepare_from_host(
    intent: &ResearchIntentV1,
    slot: &AttemptSlot,
    steamapps: &Path,
    manifest: &Path,
) -> anyhow::Result<UserLaunchRequest> {
    intent.validate()?;
    let executable = VerifiedSteamExecutable::inspect()?;
    let session = VerifiedSessionEnvironment::inspect(intent, steamapps, manifest)?;
    prepare_user_launch(intent, slot, &executable, &session)
}

/// Encode the only permitted transient command, with no shell or caller-selected arguments.
/// Both source and session witnesses must be freshly re-attested before sending the request.
pub fn prepare_user_launch(
    intent: &ResearchIntentV1,
    slot: &AttemptSlot,
    steam_executable: &VerifiedSteamExecutable,
    session: &VerifiedSessionEnvironment,
) -> anyhow::Result<UserLaunchRequest> {
    let digest = intent.digest()?;
    anyhow::ensure!(
        slot.study_sha256 == digest
            && slot.index < intent.max_attempts
            && intent.tools.iter().any(|tool| tool.source == slot.arm),
        "launch is outside the frozen schedule"
    );
    let executable = text_path(&steam_executable.0)?;
    anyhow::ensure!(
        steam_executable.0.starts_with("/nix/store"),
        "Steam executable is outside the Nix store"
    );
    anyhow::ensure!(
        session.uid == intent.uid
            && session.verified_at.elapsed() <= std::time::Duration::from_secs(3),
        "session observation is stale or belongs to another user"
    );
    let home = text_path(&session.home)?;
    let runtime_text = text_path(&session.runtime)?;
    anyhow::ensure!(
        runtime_text == format!("/run/user/{}", intent.uid),
        "wrong user runtime directory"
    );
    let name = format!(
        "stutter-research-{}-{:04}.service",
        &digest[..16],
        slot.index
    );
    let args = [executable, "-applaunch", "1091500", "-benchmark"].map(str::to_owned);
    let environment = [
        format!("HOME={home}"),
        format!("XDG_RUNTIME_DIR={runtime_text}"),
        format!("DBUS_SESSION_BUS_ADDRESS=unix:path={runtime_text}/bus"),
        "PATH=/run/current-system/sw/bin".to_owned(),
    ];
    let mut writer = Writer::default();
    writer.string(&name);
    writer.string("fail");
    array(&mut writer, 8, |writer| {
        property(writer, "Type", "s", |writer| writer.string("exec"));
        property(writer, "NoNewPrivileges", "b", |writer| writer.bool(true));
        property(writer, "CapabilityBoundingSet", "t", |writer| writer.u64(0));
        property(writer, "AmbientCapabilities", "t", |writer| writer.u64(0));
        property(writer, "KillMode", "s", |writer| {
            writer.string("control-group")
        });
        property(writer, "RuntimeMaxUSec", "t", |writer| {
            writer.u64(u64::from(intent.max_attempt_seconds) * 1_000_000)
        });
        property(writer, "Environment", "as", |writer| {
            writer.string_array(&environment)
        });
        property(writer, "ExecStart", "a(sasb)", |writer| {
            array(writer, 8, |writer| {
                writer.align(8);
                writer.string(executable);
                writer.string_array(&args);
                writer.bool(false);
            });
        });
    });
    array(&mut writer, 8, |_| {});
    anyhow::ensure!(writer.buf.len() <= 16 * 1024, "launch request is too large");
    Ok(UserLaunchRequest {
        unit_name: name,
        body: writer.buf,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::dbus::{Value, fake_reader};

    #[test]
    fn request_contains_only_the_frozen_fixed_command_and_unprivileged_properties() {
        let intent = super::super::tests::intent();
        assert_eq!(START_SIGNATURE, "ssa(sv)a(sa(sv))");
        let slot = AttemptSlot {
            study_sha256: intent.digest().unwrap(),
            context_sha256: "e".repeat(64),
            index: 0,
            arm: intent.tools[0].source,
        };
        let executable =
            VerifiedSteamExecutable(PathBuf::from("/nix/store/fixture-steam/bin/steam"));
        let session = VerifiedSessionEnvironment {
            uid: intent.uid,
            home: "/home/fixture".into(),
            runtime: format!("/run/user/{}", intent.uid).into(),
            verified_at: std::time::Instant::now(),
        };
        let request = prepare_user_launch(&intent, &slot, &executable, &session).unwrap();
        let mut reader = fake_reader(&request.body);
        assert_eq!(reader.value("s").unwrap(), Value::Str(request.unit_name));
        assert_eq!(reader.value("s").unwrap(), Value::Str("fail".into()));
        let Value::Array(properties) = reader.value("a(sv)").unwrap() else {
            panic!("properties")
        };
        let property = |name: &str| {
            properties
                .iter()
                .find_map(|entry| match entry {
                    Value::Struct(fields) if fields.first() == Some(&Value::Str(name.into())) => {
                        fields.get(1)
                    }
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(
            property("NoNewPrivileges"),
            &Value::Variant(Box::new(Value::Bool(true)))
        );
        assert_eq!(
            property("CapabilityBoundingSet"),
            &Value::Variant(Box::new(Value::U64(0)))
        );
        assert_eq!(
            property("AmbientCapabilities"),
            &Value::Variant(Box::new(Value::U64(0)))
        );
        let expected_args = [
            executable.0.to_str().unwrap(),
            "-applaunch",
            "1091500",
            "-benchmark",
        ]
        .map(|arg| Value::Str(arg.into()))
        .to_vec();
        assert_eq!(
            property("ExecStart"),
            &Value::Variant(Box::new(Value::Array(vec![Value::Struct(vec![
                Value::Str(executable.0.to_str().unwrap().into()),
                Value::Array(expected_args),
                Value::Bool(false)
            ])])))
        );
        assert_eq!(reader.value("a(sa(sv))").unwrap(), Value::Array(vec![]));
        assert_eq!(reader.pos, request.body.len());
        assert!(
            prepare_user_launch(
                &intent,
                &slot,
                &VerifiedSteamExecutable(PathBuf::from("/tmp/steam")),
                &session
            )
            .is_err()
        );
        assert!(
            prepare_user_launch(
                &intent,
                &slot,
                &executable,
                &VerifiedSessionEnvironment {
                    uid: intent.uid,
                    home: session.home.clone(),
                    runtime: "/run/user/0".into(),
                    verified_at: std::time::Instant::now()
                }
            )
            .is_err()
        );
        let mut other = slot.clone();
        other.study_sha256 = "f".repeat(64);
        assert!(prepare_user_launch(&intent, &other, &executable, &session).is_err());
        let stale = VerifiedSessionEnvironment {
            uid: intent.uid,
            home: session.home.clone(),
            runtime: session.runtime.clone(),
            verified_at: std::time::Instant::now() - std::time::Duration::from_secs(4),
        };
        assert!(prepare_user_launch(&intent, &slot, &executable, &stale).is_err());
        let mut invalid_intent = intent;
        invalid_intent.uid = 0;
        assert!(
            prepare_from_host(
                &invalid_intent,
                &slot,
                Path::new("/unused"),
                Path::new("/unused")
            )
            .is_err()
        );
    }
}
