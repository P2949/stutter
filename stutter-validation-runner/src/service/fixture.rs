//! Disposable workload ownership. These actions only narrow the product's operation policy.
use super::*;
use crate::{model::ServiceOperations, process::verify_target};

pub(super) fn config_path(user: &str) -> PathBuf {
    Path::new("/run/stutter-bounded-service-validation")
        .join(format!("autotune-bounded-{user}.toml"))
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
    ownership::atomic_write(&profiles, format!("[[profile]]\nname = \"game-isolate-render-benefit\"\n[[profile.rules]]\naffinity = \"{destination}\"\nmatch_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\"]\n").as_bytes())?;
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
    let mut pending = vec![(pid, ticks)];
    let mut seen = std::collections::BTreeSet::new();
    let mut owned = Vec::new();
    while let Some((pid, ticks)) = pending.pop() {
        if !seen.insert(pid) {
            return Err(failure("fixture descendant identity repeated"));
        }
        if seen.len() > 16 {
            return Err(failure("fixture process-tree budget exceeded"));
        }
        let proof = verify_fixture(auth, pid, ticks)?;
        let children = fs::read_to_string(proc_root.join(format!("{pid}/task/{pid}/children")))
            .map_err(|error| failure(error.to_string()))?;
        for child in children.split_whitespace() {
            let child: u32 = child
                .parse()
                .map_err(|error: std::num::ParseIntError| failure(error.to_string()))?;
            let stat = fs::read_to_string(proc_root.join(format!("{child}/stat")))
                .map_err(|error| failure(error.to_string()))?;
            let ticks = stat
                .rsplit_once(") ")
                .and_then(|(_, fields)| fields.split_whitespace().nth(19))
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(|| failure("fixture descendant has no start identity"))?;
            pending.push((child, ticks));
        }
        owned.push(proof);
    }
    // Verify the complete tree before the first migration. Each task remains identity-bound at
    // its own write; a replaced or foreign child never becomes an arbitrary root cgroup target.
    let destination = leaf(auth)?;
    for proof in owned {
        proof.revalidate(auth)?;
        crate::delegation::attach(&destination, proof.pid)
            .map_err(|error| failure(error.to_string()))?;
        proof.revalidate(auth)?;
    }
    Ok(())
}

struct FixtureProof {
    pid: u32,
    ticks: u64,
    executable: (u64, u64),
}
impl FixtureProof {
    fn revalidate(&self, auth: &Authorization) -> Result<(), Denial> {
        verify_target(Path::new("/proc"), self.pid, self.ticks, auth.owner_uid)?;
        let image = fs::metadata(format!("/proc/{}/exe", self.pid))
            .map_err(|error| failure(error.to_string()))?;
        if (image.dev(), image.ino()) != self.executable {
            return Err(failure("fixture executable identity changed"));
        }
        Ok(())
    }
}

fn verify_fixture(auth: &Authorization, pid: u32, ticks: u64) -> Result<FixtureProof, Denial> {
    let proc = Path::new("/proc").join(pid.to_string());
    verify_target(Path::new("/proc"), pid, ticks, auth.owner_uid)?;
    let image = fs::metadata(proc.join("exe")).map_err(|error| failure(error.to_string()))?;
    verify_pinned(&proc.join("exe"), &auth.xtask_sha256)?;
    let args = fs::read(proc.join("cmdline")).map_err(|error| failure(error.to_string()))?;
    if args.split(|byte| *byte == 0).nth(1) != Some(b"live-workload".as_slice()) {
        return Err((
            ErrorCode::InvalidArgument,
            "only a pinned live-workload fixture may be attached".into(),
        ));
    }
    let proof = FixtureProof {
        pid,
        ticks,
        executable: (image.dev(), image.ino()),
    };
    proof.revalidate(auth)?;
    Ok(proof)
}
