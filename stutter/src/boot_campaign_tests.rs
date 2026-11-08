use super::*;
use crate::boot_environment::{Isolation, PreemptMode};

fn env(boot: &str, kernel: &str, isolcpus: &[u32], generation: &str) -> BootEnvironment {
    BootEnvironment {
        boot_id: boot.into(),
        kernel_release: kernel.into(),
        preempt: PreemptMode::Dynamic,
        isolation: Isolation {
            isolcpus: isolcpus.to_vec(),
            ..Isolation::default()
        },
        microcode: Some("0x100".into()),
        generation: Some(generation.into()),
        scheduler_ext_ops: None,
        cmdline_hash: format!("cmd-{isolcpus:?}"),
        environment_hash: format!("env-{kernel}-{isolcpus:?}-{generation}"),
        context_hash: format!("ctx-{kernel}-{isolcpus:?}"),
    }
}

#[test]
fn a_rebuild_with_an_identical_kernel_is_comparable_and_anything_else_is_named() {
    let a = env("b1", "6.18", &[], "gen-1");
    let rebuilt = env("b2", "6.18", &[], "gen-2");
    let comparison = compare_boots(&a, &rebuilt);
    assert!(comparison.comparable());
    assert_eq!(comparison.differences.len(), 1);

    let other_kernel = compare_boots(&a, &env("b3", "7.2", &[], "gen-1"));
    assert!(!other_kernel.comparable());
    assert!(
        matches!(&other_kernel.differences[0], BootDifference::Kernel(x, y) if x == "6.18" && y == "7.2")
    );

    let isolated = compare_boots(&a, &env("b4", "6.18", &[2, 3], "gen-1"));
    assert!(
        isolated
            .differences
            .contains(&BootDifference::Isolation("isolcpus"))
    );
    assert!(!isolated.comparable());

    let mut microcode = rebuilt;
    microcode.microcode = Some("0x200".into());
    assert!(!compare_boots(&a, &microcode).comparable());
}

#[test]
fn blocks_group_by_context_never_pool_across_kernels_and_a_repeat_is_refused() {
    let mut campaign = CrossBootCampaign::new("c1");
    let a = env("b1", "6.18", &[], "gen-1");
    let rebuilt = env("b2", "6.18", &[], "gen-2");
    let other = env("b3", "7.2", &[], "gen-1");
    campaign.record(&a, "baseline", 0, 10.0).unwrap();
    campaign.record(&rebuilt, "baseline", 1, 10.5).unwrap();
    campaign.record(&other, "baseline", 0, 12.0).unwrap();
    let groups = campaign.by_context();
    assert_eq!(groups.len(), 2, "6.18 (two boots, one context) and 7.2");
    assert_eq!(groups["ctx-6.18-[]"]["baseline"].len(), 2);
    assert_eq!(groups["ctx-7.2-[]"]["baseline"].len(), 1);
    assert!(
        campaign.record(&a, "baseline", 0, 99.0).is_err(),
        "no double counting after a resume"
    );
    assert!(campaign.record(&a, "treatment", 0, f64::NAN).is_err());
}

#[test]
fn a_campaign_survives_a_reboot_as_a_file_and_a_corrupt_file_is_an_error() {
    let path = std::env::temp_dir().join(format!("stutter-campaign-{}.json", std::process::id()));
    let mut campaign = CrossBootCampaign::new("c2");
    campaign
        .record(&env("b1", "6.18", &[], "g"), "baseline", 0, 1.0)
        .unwrap();
    campaign.save(&path).unwrap();
    let mut resumed = CrossBootCampaign::load(&path).unwrap();
    assert_eq!(resumed, campaign);
    resumed
        .record(&env("b2", "6.18", &[], "g"), "baseline", 1, 2.0)
        .unwrap();
    assert_eq!(resumed.blocks.len(), 2);
    std::fs::write(&path, "not json").unwrap();
    assert!(CrossBootCampaign::load(&path).is_err());
    std::fs::remove_file(&path).unwrap();
}
