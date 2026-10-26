use std::fs;

use super::{Scratch, authorization, envelope, grant, me, observation, peer};
use crate::model::{ErrorCode, LiveSuite, Request, RunnerAction};

fn denied(result: Result<(), (ErrorCode, String)>) -> ErrorCode {
    result.unwrap_err().0
}

#[test]
fn an_authorized_peer_with_a_matching_grant_is_accepted() {
    let auth = authorization("/repo");
    auth.authorize(&peer(me()), &envelope("live-observation", observation()), 8)
        .unwrap();
}

#[test]
fn an_unknown_uid_is_rejected_before_anything_else() {
    let auth = authorization("/repo");
    let mut wrong_everything = envelope("live-observation", observation());
    wrong_everything.campaign_id = "other".to_owned();
    // Even with a wrong campaign id, the peer check comes first and leaks nothing about it.
    assert_eq!(
        denied(auth.authorize(&peer(me() + 1), &wrong_everything, 8)),
        ErrorCode::UnauthorizedPeer
    );
    assert_eq!(
        denied(auth.authorize(&peer(0), &envelope("live-observation", observation()), 8)),
        ErrorCode::UnauthorizedPeer
    );
}

#[test]
fn the_wrong_campaign_or_manifest_is_rejected() {
    let auth = authorization("/repo");
    let mut other = envelope("live-observation", observation());
    other.campaign_id = "core-other".to_owned();
    assert_eq!(
        denied(auth.authorize(&peer(me()), &other, 8)),
        ErrorCode::UnknownCampaign
    );
    let mut changed = envelope("live-observation", observation());
    changed.manifest_sha256 = "f".repeat(64);
    assert_eq!(
        denied(auth.authorize(&peer(me()), &changed, 8)),
        ErrorCode::ManifestMismatch
    );
    let mut schema = envelope("live-observation", observation());
    schema.schema_version = 99;
    assert_eq!(
        denied(auth.authorize(&peer(me()), &schema, 8)),
        ErrorCode::SchemaMismatch
    );
}

#[test]
fn unapproved_actions_and_tasks_are_rejected() {
    let auth = authorization("/repo");
    // No grant for this action at all.
    assert_eq!(
        denied(auth.authorize(
            &peer(me()),
            &envelope(
                "live-observation",
                Request::PrepareKernelCycle {
                    entry_id: "kernel-b".into()
                }
            ),
            8
        )),
        ErrorCode::ActionNotPermitted
    );
    // The action exists, but not for this task.
    assert_eq!(
        denied(auth.authorize(&peer(me()), &envelope("game-runnable", observation()), 8)),
        ErrorCode::TaskNotPermitted
    );
    // The task may run the overhead suite, not the observation suite.
    assert_eq!(
        denied(auth.authorize(
            &peer(me()),
            &envelope("overhead-controlled", observation()),
            8
        )),
        ErrorCode::TaskNotPermitted
    );
    let overhead = Request::ExecuteLiveSuite {
        suite: LiveSuite::Overhead,
        attempt: 1,
        seconds: Some(20),
        rounds: Some(12),
        benefit: None,
        mutation: None,
    };
    auth.authorize(&peer(me()), &envelope("overhead-controlled", overhead), 8)
        .unwrap();
}

#[test]
fn a_request_cannot_name_an_executable_or_extra_fields() {
    // The schema has no path, program or argument fields, and unknown fields are refused.
    let hostile = r#"{"schema_version":1,"campaign_id":"c","task_id":"t","request_id":"r","manifest_sha256":"m",
        "body":{"ExecuteLiveSuite":{"suite":"Observation","attempt":1,"seconds":20,"rounds":null,"program":"/bin/sh"}}}"#;
    assert!(serde_json::from_str::<crate::model::Envelope>(hostile).is_err());
    let extra_envelope = r#"{"schema_version":1,"campaign_id":"c","task_id":"t","request_id":"r","manifest_sha256":"m",
        "body":"Status","shell":"id"}"#;
    assert!(serde_json::from_str::<crate::model::Envelope>(extra_envelope).is_err());
    for forbidden in ["\"Shell\"", "{\"Exec\":{}}", "{\"RunCommand\":{}}"] {
        let text = format!(
            r#"{{"schema_version":1,"campaign_id":"c","task_id":"t","request_id":"r","manifest_sha256":"m","body":{forbidden}}}"#
        );
        assert!(
            serde_json::from_str::<crate::model::Envelope>(&text).is_err(),
            "{forbidden}"
        );
    }
}

#[test]
fn boot_entries_outside_the_authorization_are_rejected() {
    let auth = authorization("/repo");
    auth.authorize(
        &peer(me()),
        &envelope(
            "kernel-matrix",
            Request::SetNextBoot {
                entry_id: "kernel-b".into(),
            },
        ),
        8,
    )
    .unwrap();
    assert_eq!(
        denied(auth.authorize(
            &peer(me()),
            &envelope(
                "kernel-matrix",
                Request::SetNextBoot {
                    entry_id: "evil-entry".into()
                }
            ),
            8
        )),
        ErrorCode::BootEntryNotAuthorized
    );
}

#[test]
fn arguments_are_bounded() {
    let auth = authorization("/repo");
    let run = |body| auth.authorize(&peer(me()), &envelope("live-observation", body), 8);
    let observation = |attempt, seconds, rounds| Request::ExecuteLiveSuite {
        suite: LiveSuite::Observation,
        attempt,
        seconds,
        rounds,
        benefit: None,
        mutation: None,
    };
    assert!(run(observation(0, Some(20), None)).is_err());
    assert!(run(observation(1, Some(5), None)).is_err());
    assert!(run(observation(1, Some(61), None)).is_err());
    assert!(
        run(observation(1, None, Some(12))).is_err(),
        "only overhead takes rounds"
    );
    let burst = |cpu, burst_ms, period_ms, seconds| {
        authorization("/repo").authorize(
            &peer(me()),
            &envelope(
                "score-runnable",
                Request::ExecuteSchedulerBurst {
                    cpu,
                    burst_ms,
                    period_ms,
                    seconds,
                },
            ),
            8,
        )
    };
    burst(0, 4, 20, 30).unwrap();
    assert!(burst(8, 4, 20, 30).is_err(), "cpu outside the online set");
    assert!(
        burst(0, 20, 20, 30).is_err(),
        "burst must be below the period"
    );
    assert!(burst(0, 4, 20, 0).is_err() && burst(0, 4, 20, 4_000).is_err());
}

#[test]
fn unsafe_identifiers_never_become_paths() {
    let auth = authorization("/repo");
    for bad in ["../x", "a/b", "", ".hidden", "a b"] {
        let mut hostile = envelope("live-observation", observation());
        hostile.task_id = bad.to_owned();
        assert_eq!(
            denied(auth.authorize(&peer(me()), &hostile, 8)),
            ErrorCode::InvalidArgument,
            "{bad:?}"
        );
        let mut hostile = envelope("live-observation", observation());
        hostile.request_id = bad.to_owned();
        assert_eq!(
            denied(auth.authorize(&peer(me()), &hostile, 8)),
            ErrorCode::InvalidArgument,
            "{bad:?}"
        );
    }
}

#[test]
fn a_malformed_authorization_is_never_used() {
    let mut auth = authorization("/repo");
    assert!(auth.validate_shape().is_ok());
    for mutate in [
        (|a: &mut crate::model::Authorization| a.owner_uid = 0) as fn(&mut _),
        |a| a.repository = "relative/path".to_owned(),
        |a| a.repository = "/repo/../etc".to_owned(),
        |a| a.manifest_sha256 = "short".to_owned(),
        |a| a.campaign_id = "../escape".to_owned(),
        |a| a.schema_version = 7,
        |a| a.allowed_boot_entries.push("a/b".to_owned()),
        |a| {
            a.grants
                .insert(grant("t", RunnerAction::ExecuteLiveSuite, None));
        },
        |a| {
            a.grants.insert(grant(
                "t",
                RunnerAction::Status,
                Some(LiveSuite::Observation),
            ));
        },
    ] {
        let mut broken = auth.clone();
        mutate(&mut broken);
        assert!(broken.validate_shape().is_err());
    }
    auth.owner_uid = me();
    assert!(auth.validate_shape().is_ok());
}

#[test]
fn the_authorization_file_must_be_root_owned_and_tamper_proof_when_strict() {
    let scratch = Scratch::new("auth-file");
    let path = scratch.path().join("authorization.json");
    fs::write(&path, serde_json::to_vec(&authorization("/repo")).unwrap()).unwrap();
    // Owned by the campaign user: acceptable only for unprivileged development use.
    if crate::sys::effective_uid() != 0 {
        let error = crate::model::Authorization::load(&path, true).unwrap_err();
        assert!(error.contains("root-owned"), "{error}");
    }
    assert!(crate::model::Authorization::load(&path, false).is_ok());
    fs::write(&path, b"{ not json").unwrap();
    assert!(
        crate::model::Authorization::load(&path, false)
            .unwrap_err()
            .contains("corrupt")
    );
    let link = scratch.path().join("link.json");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(
        crate::model::Authorization::load(&link, false).is_err(),
        "symlinks are refused"
    );
}

#[test]
fn the_benefit_suite_needs_its_spec_and_nothing_else_may_carry_one() {
    use crate::model::{BenefitScenario, BenefitSpec};
    let spec = BenefitSpec {
        scenario: BenefitScenario::Beneficial,
        pair: 3,
        baseline_first: true,
    };
    let mut auth = authorization("/work/stutter");
    auth.grants.insert(grant(
        "benefit_calibration",
        RunnerAction::ExecuteLiveSuite,
        Some(LiveSuite::AffinityBenefit),
    ));
    let request = |suite, benefit| Request::ExecuteLiveSuite {
        suite,
        attempt: 1,
        seconds: Some(20),
        rounds: None,
        benefit,
        mutation: None,
    };
    let authorize =
        |request| auth.authorize(&peer(me()), &envelope("benefit_calibration", request), 8);
    assert!(authorize(request(LiveSuite::AffinityBenefit, Some(spec))).is_ok());
    assert_eq!(
        authorize(request(LiveSuite::AffinityBenefit, None))
            .unwrap_err()
            .0,
        ErrorCode::InvalidArgument
    );
    let too_far = BenefitSpec {
        pair: 10_000,
        ..spec
    };
    assert_eq!(
        authorize(request(LiveSuite::AffinityBenefit, Some(too_far)))
            .unwrap_err()
            .0,
        ErrorCode::InvalidArgument
    );
    // A different suite never takes the spec (and is not granted to this task anyway).
    assert!(authorize(request(LiveSuite::Observation, Some(spec))).is_err());
}

#[test]
fn only_the_recovery_lifecycle_and_benefit_suites_take_a_mutation() {
    use crate::model::Mutation;
    let mut auth = authorization("/work/stutter");
    for suite in [LiveSuite::Observation, LiveSuite::Recovery] {
        auth.grants
            .insert(grant("t", RunnerAction::ExecuteLiveSuite, Some(suite)));
    }
    let request = |suite, mutation| Request::ExecuteLiveSuite {
        suite,
        attempt: 1,
        seconds: None,
        rounds: None,
        benefit: None,
        mutation,
    };
    let authorize = |request| auth.authorize(&peer(me()), &envelope("t", request), 8);
    assert!(authorize(request(LiveSuite::Recovery, Some(Mutation::Uclamp))).is_ok());
    assert_eq!(
        authorize(request(LiveSuite::Observation, Some(Mutation::Uclamp)))
            .unwrap_err()
            .0,
        ErrorCode::InvalidArgument
    );
}
