//! Disposable workload ownership. These actions only narrow the product's operation policy.
use super::*;
use crate::{model::ServiceOperations, process::verify_target};

pub(super) fn config_path(user: &str) -> PathBuf {
    Path::new("/etc/stutter").join(format!("autotune-bounded-{user}.toml"))
}

fn leaf(auth: &Authorization) -> Result<PathBuf, Denial> {
    crate::delegation::prepare(
        Path::new("/sys/fs/cgroup"),
        &auth.campaign_id,
        Some((auth.owner_uid, auth.owner_gid)),
    )
    .map_err(|error| failure(error.to_string()))
}

pub(super) fn prepare(
    auth: &Authorization,
    user: &str,
    source: u32,
    destination: u32,
    operations: ServiceOperations,
) -> Result<(), Denial> {
    if source == destination
        || [source, destination].iter().any(|cpu| {
            !Path::new("/sys/devices/system/cpu")
                .join(format!("cpu{cpu}"))
                .is_dir()
        })
    {
        return Err((
            ErrorCode::InvalidArgument,
            "fixture requires two distinct present CPUs".into(),
        ));
    }
    // Configuration changes happen only while both real service actors are stopped.
    for component in [ServiceComponent::Controller, ServiceComponent::Worker] {
        let unit = unit_name(user, component)?;
        let pid = systemctl(&["show", "--property=MainPID", "--value", &unit])?;
        if pid.trim() != "0" {
            return Err(failure(
                "stop bounded service actors before fixture configuration",
            ));
        }
    }
    let suite = leaf(auth)?;
    let parent = suite
        .parent()
        .ok_or_else(|| failure("missing campaign cgroup"))?;
    let target = parent.join("target");
    if !target.exists() {
        fs::create_dir(&target).map_err(|error| failure(error.to_string()))?;
    }
    fs::write(
        target.join("cpuset.mems"),
        fs::read_to_string(parent.join("cpuset.mems.effective"))
            .map_err(|error| failure(error.to_string()))?,
    )
    .and_then(|()| fs::write(target.join("cpuset.cpus"), destination.to_string()))
    .map_err(|error| failure(error.to_string()))?;
    let denied = match operations {
        ServiceOperations::Affinity => "daemon_denied_action_families = [\"cgroup_placement\"]\n",
        ServiceOperations::Cgroup => "daemon_denied_action_families = [\"cpu_affinity_profile\"]\n",
        ServiceOperations::Both => "",
    };
    let profiles = config_path(user).with_extension("profiles.toml");
    ownership::atomic_write(&profiles, format!("[[profile]]\nname = \"game-isolate-render-benefit\"\n[[profile.rules]]\naffinity = \"{destination}\"\nmatch_class = [\"GameScope\", \"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\"]\n").as_bytes())?;
    let relative = target
        .strip_prefix("/sys/fs/cgroup")
        .map_err(|error| failure(error.to_string()))?;
    ownership::atomic_write(&config_path(user), format!(
        "daemon_game_cgroup = \"/{}\"\n{denied}\n[autotune]\nworker_only_mutation = true\nmanage_privileged_worker = false\nprivileged_worker_socket = \"/run/stutter-worker-{user}/worker.sock\"\nvalidation_workload_cgroup = \"{}\"\nprofiles = \"{}\"\n", relative.display(), parent.display(), profiles.display()).as_bytes())?;
    ownership::record(auth, user)
}

pub(super) fn attach(
    auth: &Authorization,
    _user: &str,
    pid: u32,
    ticks: u64,
) -> Result<(), Denial> {
    let proc_root = Path::new("/proc");
    verify_target(proc_root, pid, ticks, auth.owner_uid)?;
    let proc = proc_root.join(pid.to_string());
    // Only the pinned workload fixture may be migrated; arbitrary desktop processes are refused.
    verify_pinned(&proc.join("exe"), &auth.xtask_sha256)?;
    let args = fs::read(proc.join("cmdline")).map_err(|error| failure(error.to_string()))?;
    if args.split(|byte| *byte == 0).nth(1) != Some(b"live-workload".as_slice()) {
        return Err((
            ErrorCode::InvalidArgument,
            "only a pinned live-workload fixture may be attached".into(),
        ));
    }
    let destination = leaf(auth)?;
    verify_target(proc_root, pid, ticks, auth.owner_uid)?;
    crate::delegation::attach(&destination, pid).map_err(|error| failure(error.to_string()))?;
    verify_target(proc_root, pid, ticks, auth.owner_uid)
}
