use std::fs;

use super::*;

/// A fake `/sys/block` whose `queue/scheduler` behaves like the kernel's: writing a supported name
/// makes it the bracketed one.
struct Fake {
    root: PathBuf,
}

fn fake(name: &str) -> Fake {
    let root = std::env::temp_dir().join(format!("stutter-blk-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for dev in ["disk1", "rootdisk"] {
        fs::create_dir_all(root.join(dev).join("queue")).unwrap();
        fs::write(root.join(dev).join("dev"), "259:1\n").unwrap();
        fs::write(
            root.join(dev).join("queue/scheduler"),
            "none [mq-deadline] kyber bfq\n",
        )
        .unwrap();
    }
    fs::write(root.join("rootdisk/dev"), "259:0\n").unwrap();
    Fake { root }
}

/// The kernel stand-in: set the bracket to `name`.
fn kernel_select(queue: &Path, name: &str) {
    let text = ["none", "mq-deadline", "kyber", "bfq"]
        .iter()
        .map(|s| {
            if *s == name {
                format!("[{s}]")
            } else {
                (*s).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    fs::write(queue, format!("{text}\n")).unwrap();
}

fn request(device: &str, scheduler: &str) -> BlockSchedulerRequest {
    BlockSchedulerRequest {
        device: device.into(),
        scheduler: scheduler.into(),
        authorized: ["disk1".to_owned(), "rootdisk".to_owned()].into(),
        protected: ["rootdisk".to_owned()].into(),
    }
}

#[test]
fn a_supported_scheduler_on_an_authorized_device_commits_and_restores() {
    let fake = fake("cycle");
    let prepared = prepare(&fake.root, &request("disk1", "bfq")).unwrap();
    let queue = fake.root.join("disk1/queue/scheduler");
    assert!(
        fs::read_to_string(&queue)
            .unwrap()
            .contains("[mq-deadline]"),
        "prepare writes nothing"
    );
    let committed = prepared.commit(&Kernelish).unwrap();
    assert!(fs::read_to_string(&queue).unwrap().contains("[bfq]"));
    assert_eq!(committed.record().original, "mq-deadline");
    assert_eq!(
        restore_recorded(&fake.root, &committed.record(), &Kernelish),
        BlockSchedulerRestore::Restored
    );
    assert!(
        fs::read_to_string(&queue)
            .unwrap()
            .contains("[mq-deadline]")
    );
}

#[test]
fn unauthorized_protected_unsupported_and_unchanged_requests_are_denied_before_any_write() {
    let fake = fake("deny");
    assert_eq!(
        prepare(&fake.root, &request("rootdisk", "bfq")).unwrap_err(),
        BlockSchedulerDenial::Protected
    );
    let mut unauthorized = request("disk1", "bfq");
    unauthorized.authorized.clear();
    assert_eq!(
        prepare(&fake.root, &unauthorized).unwrap_err(),
        BlockSchedulerDenial::NotAuthorized
    );
    assert_eq!(
        prepare(&fake.root, &request("disk1", "cfq")).unwrap_err(),
        BlockSchedulerDenial::UnsupportedScheduler
    );
    assert_eq!(
        prepare(&fake.root, &request("disk1", "mq-deadline")).unwrap_err(),
        BlockSchedulerDenial::NoChange
    );
    assert_eq!(
        prepare(&fake.root, &request("../etc", "bfq")).unwrap_err(),
        BlockSchedulerDenial::BadName
    );
    assert_eq!(
        prepare(&fake.root, &request("nodisk", "bfq")).unwrap_err(),
        BlockSchedulerDenial::Unavailable
    );
    assert!(
        fs::read_to_string(fake.root.join("disk1/queue/scheduler"))
            .unwrap()
            .contains("[mq-deadline]")
    );
}

#[test]
fn restore_never_touches_a_different_device_or_a_foreign_choice() {
    let fake = fake("identity");
    let prepared = prepare(&fake.root, &request("disk1", "bfq")).unwrap();
    let record = prepared.record.clone();
    let queue = fake.root.join("disk1/queue/scheduler");
    kernel_select(&queue, "bfq");
    // Another owner picks kyber: left alone.
    kernel_select(&queue, "kyber");
    assert_eq!(
        restore_recorded(&fake.root, &record, &Kernelish),
        BlockSchedulerRestore::ExternallyChanged
    );
    assert!(fs::read_to_string(&queue).unwrap().contains("[kyber]"));
    // The name now refers to a different device number: unsafe.
    kernel_select(&queue, "bfq");
    fs::write(fake.root.join("disk1/dev"), "8:0\n").unwrap();
    assert_eq!(
        restore_recorded(&fake.root, &record, &Kernelish),
        BlockSchedulerRestore::UnsafeToRestore
    );
    assert!(fs::read_to_string(&queue).unwrap().contains("[bfq]"));
}

/// Real files with the kernel's behaviour: a write selects that scheduler and brackets it.
struct Kernelish;

impl BlockFiles for Kernelish {
    fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        let name = text.trim();
        if !["none", "mq-deadline", "kyber", "bfq"].contains(&name) {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        kernel_select(path, name);
        Ok(())
    }
}
