//! Reboot-resume of the normal campaign controller. After a reboot the interactive agent is
//! gone, so the resume service restarts the controller. It may execute exactly one thing: the
//! authorized repository's pinned `xtask validation-campaign continue --resume`, started only
//! after dropping root. Mutable repository code never runs with root privileges.
use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{
    authorization::Denial,
    credentials::{Identity, drop_all_privileges},
    model::{Authorization, ErrorCode},
    process::{verify_pinned, xtask_source_path},
    state::StateDir,
};

/// The only command the resume service ever launches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumePlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

pub fn plan_resume(auth: &Authorization) -> ResumePlan {
    let campaign = Path::new(&auth.repository)
        .join(auth.controller.campaign_root())
        .join(&auth.campaign_id);
    let [command, subcommand] = auth.controller.resume_subcommand();
    ResumePlan {
        program: xtask_source_path(auth),
        args: vec![
            command.to_owned(),
            subcommand.to_owned(),
            "--campaign".to_owned(),
            campaign.to_string_lossy().into_owned(),
            "--resume".to_owned(),
        ],
        cwd: PathBuf::from(&auth.repository),
    }
}

/// Waits until the runner's socket exists, so a resumed controller never races a runner that
/// systemd has started but that is not serving yet. `sleep` is injectable for tests.
pub fn wait_for_runner(socket: &Path, attempts: u32, sleep: &dyn Fn(std::time::Duration)) -> bool {
    for attempt in 0..attempts {
        if socket.exists() {
            return true;
        }
        if attempt + 1 < attempts {
            sleep(std::time::Duration::from_secs(1));
        }
    }
    socket.exists()
}

/// `HOME` of the resumed controller: the campaign owner's real home, so cargo's caches land in
/// `~/.cargo` and not inside the repository (an untracked `.cargo` there dirties the tree the
/// campaign requires to be clean). Falls back to the repository when the owner has no entry.
pub fn controller_home(passwd: &str, uid: u32, repository: &Path) -> PathBuf {
    crate::install::home_of(passwd, uid).unwrap_or_else(|| repository.to_path_buf())
}

/// Starts the controller for one campaign, detached, as the campaign user with no capabilities.
pub fn resume_controller(auth: &Authorization, drop_to_owner: bool) -> Result<u32, Denial> {
    verify_pinned(&xtask_source_path(auth), &auth.xtask_sha256)?;
    let plan = plan_resume(auth);
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let home = controller_home(&passwd, auth.owner_uid, &plan.cwd);
    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .env_clear()
        .env("PATH", &auth.path_env)
        .env("HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    drop_all_privileges(
        &mut command,
        drop_to_owner.then_some(Identity {
            uid: auth.owner_uid,
            gid: auth.owner_gid,
        }),
    );
    // New session so the controller survives the oneshot unit finishing.
    command.process_group(0);
    command.spawn().map(|child| child.id()).map_err(|error| {
        (
            ErrorCode::ExecutionFailed,
            format!("could not resume the controller: {error}"),
        )
    })
}

/// Resumes the one campaign `state_root/current` names, if its runner state says a reboot is in
/// flight. That symlink is the only selector: nothing is enumerated, so a campaign can never be
/// reached twice, and a missing, broken or escaping selector resumes nothing. Corrupt or
/// cancelled campaigns are skipped and reported, never resumed.
pub fn resume_all(state_root: &Path, drop_to_owner: bool) -> Vec<String> {
    let mut report = Vec::new();
    let dirs = match current_campaign(state_root) {
        Ok(dir) => vec![dir],
        Err(why) => {
            report.push(format!("no campaign selected: {why}"));
            return report;
        }
    };
    for dir in dirs
        .into_iter()
        .filter(|dir| dir.join("authorization.json").is_file())
    {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_owned();
        let auth = match Authorization::load(&dir.join("authorization.json"), drop_to_owner) {
            Ok(auth) => auth,
            Err(error) => {
                report.push(format!("{name}: skipped, authorization rejected: {error}"));
                continue;
            }
        };
        let state = match (StateDir(dir.clone())).load(&auth.campaign_id) {
            Ok(state) => state,
            Err(error) => {
                report.push(format!("{name}: skipped, {error}"));
                continue;
            }
        };
        if state.cancelled || !state.resume_pending {
            report.push(format!("{name}: nothing to resume"));
            continue;
        }
        match resume_controller(&auth, drop_to_owner) {
            Ok(pid) => report.push(format!("{name}: controller resumed (pid {pid})")),
            Err((_, message)) => report.push(format!("{name}: {message}")),
        }
    }
    report
}

/// The single campaign directory `current` selects: a direct child of the canonical state root.
fn current_campaign(state_root: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(state_root).map_err(|e| format!("state root: {e}"))?;
    let selected = fs::canonicalize(state_root.join("current"))
        .map_err(|e| format!("current selector unusable: {e}"))?;
    if selected.parent() != Some(root.as_path()) {
        return Err("current does not name a campaign directly under the state root".to_owned());
    }
    Ok(selected)
}
