use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

use super::{Scratch, authorization, me};
use crate::{
    model::Authorization,
    resume::{plan_resume, resume_all},
    state::{RunnerState, StateDir},
};

#[test]
fn the_resume_plan_is_exactly_one_pinned_command() {
    let auth = authorization("/work/stutter");
    let plan = plan_resume(&auth);
    assert_eq!(
        plan.program,
        std::path::Path::new("/work/stutter/target/debug/xtask")
    );
    assert_eq!(
        plan.args,
        [
            "validation-campaign",
            "continue",
            "--campaign",
            "/work/stutter/.llm-runs/validation/core-abc1234-1",
            "--resume",
        ]
    );
    assert_eq!(plan.cwd, std::path::Path::new("/work/stutter"));
}

fn campaign(
    scratch: &Scratch,
    body: &str,
    resume_pending: bool,
    cancelled: bool,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = scratch.path().join("repo");
    fs::create_dir_all(repo.join("target/debug")).unwrap();
    let xtask = repo.join("target/debug/xtask");
    fs::write(&xtask, body).unwrap();
    fs::set_permissions(&xtask, fs::Permissions::from_mode(0o755)).unwrap();
    let mut auth: Authorization = authorization(&repo.to_string_lossy());
    auth.xtask_sha256 = stutter_core::hash::sha256_hex(body.as_bytes());
    let state_root = scratch.path().join("state");
    let dir = state_root.join(&auth.campaign_id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("authorization.json"),
        serde_json::to_vec(&auth).unwrap(),
    )
    .unwrap();
    let state = StateDir(dir);
    state
        .save(&RunnerState {
            schema_version: 1,
            campaign_id: auth.campaign_id.clone(),
            resume_pending,
            cancelled,
            ..RunnerState::default()
        })
        .unwrap();
    (state_root, repo)
}

#[test]
fn a_pending_campaign_is_resumed_by_exec_of_the_pinned_controller_only() {
    if crate::sys::effective_uid() == 0 {
        eprintln!("skipped: the non-root resume path is exercised by the unprivileged run");
        return;
    }
    let scratch = Scratch::new("resume-run");
    let marker = scratch.path().join("marker");
    let script = format!("#!/bin/sh\necho \"$@\" > {}\n", marker.display());
    let (state_root, repo) = campaign(&scratch, &script, true, false);
    let report = resume_all(&state_root, false);
    assert!(report[0].contains("controller resumed"), "{report:?}");
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let args = fs::read_to_string(&marker).unwrap();
    assert!(
        args.starts_with("validation-campaign continue --campaign"),
        "{args}"
    );
    assert!(args.trim_end().ends_with("--resume"), "{args}");
    assert!(args.contains(&repo.to_string_lossy().into_owned()));
    let _ = me();
}

#[test]
fn cancelled_idle_corrupt_or_tampered_campaigns_are_never_resumed() {
    let scratch = Scratch::new("resume-skip");
    let marker = scratch.path().join("marker");
    let script = format!("#!/bin/sh\necho ran > {}\n", marker.display());
    // Cancelled.
    let (root, _) = campaign(&scratch, &script, true, true);
    assert!(resume_all(&root, false)[0].contains("nothing to resume"));
    // No reboot in flight.
    let idle = Scratch::new("resume-idle");
    let (root, _) = campaign(&idle, &script, false, false);
    assert!(resume_all(&root, false)[0].contains("nothing to resume"));
    // Corrupt runner state.
    let corrupt = Scratch::new("resume-corrupt");
    let (root, _) = campaign(&corrupt, &script, true, false);
    let dir = root.join("core-abc1234-1");
    fs::write(dir.join("boot-state.json"), b"{ not json").unwrap();
    assert!(resume_all(&root, false)[0].contains("corrupt"));
    // A controller whose digest no longer matches the pin is refused before it can run.
    let tampered = Scratch::new("resume-tampered");
    let (root, repo) = campaign(&tampered, &script, true, false);
    fs::write(repo.join("target/debug/xtask"), "#!/bin/sh\necho evil\n").unwrap();
    let report = resume_all(&root, false);
    assert!(report[0].contains("pinned digest"), "{report:?}");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!marker.exists(), "no controller ran in any skipped case");
}

#[test]
fn the_resume_service_waits_for_the_runner_socket_and_gives_up_after_its_budget() {
    use std::cell::Cell;

    use crate::resume::wait_for_runner;
    let scratch = Scratch::new("resume-wait");
    let socket = scratch.path().join("runner.sock");
    let slept = Cell::new(0_u32);
    // The socket appears during the third wait.
    let appeared = scratch.path().join("runner.sock");
    let sleep = |_: std::time::Duration| {
        slept.set(slept.get() + 1);
        if slept.get() == 3 {
            std::fs::write(&appeared, b"").unwrap();
        }
    };
    assert!(wait_for_runner(&socket, 10, &sleep));
    assert_eq!(slept.get(), 3);
    std::fs::remove_file(&socket).unwrap();
    let never = std::cell::Cell::new(0_u32);
    assert!(!wait_for_runner(&socket, 4, &|_| never.set(never.get() + 1)));
    assert_eq!(never.get(), 3, "one sleep fewer than the attempts");
    assert!(!wait_for_runner(&scratch.path().join("absent"), 0, &|_| {}));
}

#[test]
fn the_resumed_controller_gets_the_owners_home_not_the_repository() {
    use std::path::Path;

    use crate::resume::controller_home;
    let passwd = "root:x:0:0::/root:/bin/sh\nme:x:1000:100::/home/me:/bin/sh\n";
    assert_eq!(
        controller_home(passwd, 1000, Path::new("/home/me/Development/stutter")),
        Path::new("/home/me")
    );
    assert_eq!(
        controller_home(passwd, 4242, Path::new("/work/stutter")),
        Path::new("/work/stutter"),
        "an owner without a passwd entry falls back to the repository"
    );
}
