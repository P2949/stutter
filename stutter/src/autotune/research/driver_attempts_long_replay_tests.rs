use super::*;

/// A qualified action's `stage_evidence` is rebuilt for replay from the sealed receipts and the schedule, so the
/// shared judgement accepts the live qualification; forged, missing, partial or misattributed receipts do not.
#[test]
fn the_replayed_action_evidence_needs_a_valid_receipt_on_every_treated_attempt_of_its_own_process()
{
    use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

    use crate::autotune::research::{
        child_judgement::action_replay, report::AttemptEvidenceV1, treatment::treatments_of,
    };
    let (root, mut runner, study, gate, _parent) = build(&unit().definition_sha256);
    let outcome = run_acting(&mut runner, &study, &gate);
    assert_eq!(judge(&study, &outcome).result, StudyResult::Unqualified);
    let path = root.path().join("studies").join(&study.study_sha256[..16]);
    let dir = std::fs::File::open(&path).unwrap();
    let uid = crate::syscall::geteuid();
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&std::fs::read(path.join("intent.json")).unwrap()).unwrap();
    let schedule = treatments_of(&intent).unwrap().unwrap();
    // One distinct game process per attempt, as a real campaign has.
    let evidence: BTreeMap<u32, AttemptEvidenceV1> = (0..schedule.len() as u32)
        .map(|slot| {
            let mut record = session(&unit().definition_sha256);
            record.process = SessionProcess {
                pid: stutter_core::ids::Pid::new(5000 + slot),
                start_ticks: 100 + u64::from(slot),
                uid,
            };
            let item = AttemptEvidenceV1 {
                schema_version: 1,
                slot_index: slot,
                average_fps: 1.0,
                native_completion: None,
                long_session: Some(Box::new(record)),
                thermal: None,
            };
            (slot, item)
        })
        .collect();
    let state = |pid: u32, ticks: u64| {
        serde_json::json!({
            "cgroup": "0::/app.slice", "cpus_allowed": "0-15",
            "tasks": { pid.to_string(): "0-15" }, "identity": [ticks, uid],
        })
    };
    let receipt = |slot: u32| {
        let (pid, ticks) = (5000 + slot, 100 + u64::from(slot));
        serde_json::json!({
            "transaction": format!("exp-{slot}"), "action": "CpuAffinityProfile",
            "pid": pid, "start_ticks": ticks, "uid": uid,
            "permitted_unix": 5_100 + 10 * u64::from(slot), "applied_unix": 5_101 + 10 * u64::from(slot),
            "settled_unix": 5_102 + 10 * u64::from(slot), "peak_tokens": 1,
            "original": state(pid, ticks), "restored": state(pid, ticks),
        })
    };
    let treated: Vec<u32> = schedule
        .iter()
        .enumerate()
        .filter(|(_, t)| **t == crate::autotune::research::Treatment::On)
        .map(|(slot, _)| slot as u32)
        .collect();
    let untreated = (0..schedule.len() as u32)
        .find(|slot| !treated.contains(slot))
        .unwrap();
    // The controller's own journal of the same transactions, archived beside the receipts the gate wrote.
    let journal_of = |items: &[serde_json::Value]| -> Vec<serde_json::Value> {
        items
            .iter()
            .map(|item| {
                crate::autotune::research::evidence_producers::controller_journal_record(
                    item["transaction"].as_str().unwrap(),
                    "cpu-affinity-profile:game-main",
                    (
                        u32::try_from(item["pid"].as_u64().unwrap()).unwrap(),
                        item["start_ticks"].as_u64().unwrap(),
                    ),
                    (5_101, 5_102),
                )
            })
            .collect()
    };
    let put = |name: &str, value: &Vec<serde_json::Value>| {
        let file = path.join(name);
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
        std::fs::write(&file, serde_json::to_vec(value).unwrap()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    // The privileged worker's own export of the same transactions, each on the receipt's own process and at its own
    // seconds. `unrestored` leaves that transaction without its restoration; `foreign` adds an applied transaction on
    // another process that no receipt accounts for.
    let worker_of = |items: &[serde_json::Value],
                     unrestored: Option<usize>,
                     foreign: bool|
     -> Vec<serde_json::Value> {
        let action = "cpu-affinity-profile:game-main";
        let mut export = crate::autotune::research::evidence_producers::WorkerExport::default();
        for (index, item) in items.iter().enumerate() {
            let process = (
                u32::try_from(item["pid"].as_u64().unwrap_or(0)).unwrap_or(0),
                item["start_ticks"].as_u64().unwrap_or(0),
            );
            let name = crate::autotune::research::evidence_producers::worker_transaction_of(
                item["transaction"].as_str().unwrap(),
            );
            let (permitted, applied, settled) = (
                item["permitted_unix"].as_u64().unwrap_or(0),
                item["applied_unix"].as_u64().unwrap_or(0),
                item["settled_unix"].as_u64().unwrap_or(0),
            );
            if unrestored == Some(index) {
                export.push("prepared", &name, action, process, permitted);
                export.push("applied", &name, action, process, applied);
            } else {
                export.transaction(&name, action, process, (permitted, applied, settled));
            }
        }
        if foreign {
            export.transaction("tx-foreign", action, (4_242, 5), (5_150, 5_150, 5_151));
        }
        export.records().to_vec()
    };
    let write = |items: Vec<serde_json::Value>| {
        put("action-receipts.json", &items);
        put(
            crate::autotune::research::controller_provenance::FILE,
            &journal_of(&items),
        );
        put(
            crate::autotune::research::worker_provenance::FILE,
            &worker_of(&items, None, false),
        );
    };
    let replay = || {
        action_replay(
            &dir,
            uid,
            uid,
            ResearchAction::CpuAffinityProfile,
            &intent,
            &evidence,
        )
    };

    write(treated.iter().map(|slot| receipt(*slot)).collect());
    assert!(replay(), "every treated attempt has its own valid receipt");
    // The worker-transaction timeline: missing, out of order, no worker token, or outside the attempt each fail.
    let with = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut items: Vec<_> = treated.iter().map(|slot| receipt(*slot)).collect();
        edit(&mut items[0]);
        write(items);
        replay()
    };
    assert!(
        !with(&|r| {
            r.as_object_mut().unwrap().remove("settled_unix");
        }),
        "no timeline"
    );
    assert!(
        !with(&|r| r["applied_unix"] = 5_200.into()),
        "applied after it settled"
    );
    assert!(
        !with(&|r| r["peak_tokens"] = 0.into()),
        "the root worker never held a token"
    );
    assert!(
        !with(&|r| r["permitted_unix"] = 4_000.into()),
        "permitted before the attempt began"
    );
    assert!(
        !with(&|r| r["settled_unix"] = 7_000.into()),
        "settled after the attempt ended"
    );
    write(treated.iter().map(|slot| receipt(*slot)).collect());
    // A worker thread that exited while the action ran: valid only when the receipt names it, the post-restore
    // listing no longer has it, and it is not the leader.
    let with_exit = |named: Option<serde_json::Value>, still_listed: bool| {
        treated
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let mut item = receipt(*slot);
                if index == 0 {
                    let tid = 9_000 + slot;
                    item["original"]["tasks"][tid.to_string()] = "0-15".into();
                    if still_listed {
                        item["restored"]["tasks"][tid.to_string()] = "0-15".into();
                    }
                    if let Some(named) = &named {
                        item["exited"] = named.clone();
                    }
                }
                item
            })
            .collect::<Vec<_>>()
    };
    let first_tid = 9_000 + treated[0];
    write(with_exit(Some(serde_json::json!([first_tid])), false));
    assert!(replay(), "a named, witnessed exit is not a difference");
    write(with_exit(None, false));
    assert!(!replay(), "an unnamed absence is unknown, never an exit");
    write(with_exit(Some(serde_json::json!([first_tid])), true));
    assert!(!replay(), "a thread still listed has not exited");
    write(with_exit(
        Some(serde_json::json!([5000 + treated[0]])),
        false,
    ));
    assert!(!replay(), "the leader cannot be the thread that exited");
    write(treated.iter().map(|slot| receipt(*slot)).collect());
    // One treated attempt without a receipt is a partly applied action.
    write(treated[1..].iter().map(|slot| receipt(*slot)).collect());
    assert!(!replay());
    // A receipt on an attempt the schedule left untreated is not an applied action.
    let mut extra: Vec<_> = treated.iter().map(|slot| receipt(*slot)).collect();
    extra.push(receipt(untreated));
    write(extra);
    assert!(!replay());
    // A receipt whose restored placement differs is not restoration.
    let mut broken = treated
        .iter()
        .map(|slot| receipt(*slot))
        .collect::<Vec<_>>();
    broken[0]["restored"]["cpus_allowed"] = "0-3".into();
    write(broken);
    assert!(!replay());
    // The gate's receipts alone are not enough: the controller's own journal must corroborate every one.
    let whole: Vec<_> = treated.iter().map(|slot| receipt(*slot)).collect();
    write(whole.clone());
    assert!(replay());
    let mut journal = journal_of(&whole);
    journal.pop();
    put(
        crate::autotune::research::controller_provenance::FILE,
        &journal,
    );
    assert!(!replay(), "a transaction the controller never journaled");
    let mut journal = journal_of(&whole);
    journal[0]["state"] = "faulted".into();
    put(
        crate::autotune::research::controller_provenance::FILE,
        &journal,
    );
    assert!(!replay(), "a transaction the controller did not restore");
    // The worker's own export must corroborate too: no restoration of one transaction, an apply nobody accounts for, or no
    // audit at all each refuse the replay.
    write(whole.clone());
    assert!(replay());
    let audit = worker_of(&whole, Some(0), false);
    put(crate::autotune::research::worker_provenance::FILE, &audit);
    assert!(!replay(), "a transaction the worker never restored");
    let audit = worker_of(&whole, None, true);
    put(crate::autotune::research::worker_provenance::FILE, &audit);
    assert!(
        !replay(),
        "an apply the worker made that no receipt accounts for"
    );
    std::fs::remove_file(path.join(crate::autotune::research::worker_provenance::FILE)).unwrap();
    assert!(!replay(), "no worker audit at all");
    write(whole.clone());
    std::fs::remove_file(path.join(crate::autotune::research::controller_provenance::FILE))
        .unwrap();
    assert!(!replay(), "no journal archive at all");
    // No receipts file at all.
    std::fs::remove_file(path.join("action-receipts.json")).unwrap();
    assert!(!replay());
}
