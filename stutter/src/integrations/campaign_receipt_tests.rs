use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;

const BOOT: &str = "11111111-2222-3333-4444-555555555555";
const NOW: u64 = 1_000_000;

fn me() -> u32 {
    crate::syscall::geteuid()
}

/// A private tree standing in for the root-owned runner state: owned by the test user, 0755.
fn tree(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-receipt-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("campaign")).unwrap();
    for dir in [root.clone(), root.join("campaign")] {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
    }
    root
}

fn receipt(root: &Path, body: &str) -> PathBuf {
    let path = root.join("campaign/receipt.json");
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    path
}

fn good(scope: &str) -> String {
    format!(
        r#"{{"campaign_id":"w19-power","scopes":["{scope}"],"owner_uid":{},"boot_id":"{BOOT}","expires_unix":{}}}"#,
        me(),
        NOW + 60
    )
}

fn mint(
    root: &Path,
    receipt: &Path,
    scope: CampaignScope,
) -> Result<OfflineAuthorization, MintRefusal> {
    mint_with(
        receipt,
        &root.join("journal.jsonl"),
        scope,
        &HostFacts {
            now_unix: NOW,
            boot_id: BOOT,
            caller_uid: me(),
            anchor: root,
            trusted_uid: me(),
        },
    )
}

#[test]
fn a_valid_receipt_mints_an_authorization_and_the_issue_is_journalled_first() {
    let root = tree("ok");
    let path = receipt(&root, &good("power_profile_hold"));
    let authorization = mint(&root, &path, CampaignScope::PowerProfileHold).unwrap();
    assert_eq!(authorization.campaign(), "w19-power");
    let journal = fs::read_to_string(root.join("journal.jsonl")).unwrap();
    assert!(
        journal.contains("w19-power") && journal.contains("power_profile_hold"),
        "{journal}"
    );
    assert_eq!(journal.lines().count(), 1);
}

#[test]
fn every_check_refuses_with_its_own_reason() {
    let root = tree("refuse");
    let ok = receipt(&root, &good("power_profile_hold"));
    // Another scope than the one granted.
    assert_eq!(
        mint(&root, &ok, CampaignScope::SchedulerSwitch).unwrap_err(),
        MintRefusal::ScopeNotGranted
    );
    let cases = [
        (
            good("power_profile_hold").replace(&(NOW + 60).to_string(), &NOW.to_string()),
            MintRefusal::Expired,
        ),
        (
            good("power_profile_hold").replace(BOOT, "other-boot"),
            MintRefusal::WrongBoot,
        ),
        (
            good("power_profile_hold")
                .replace(&format!("\"owner_uid\":{}", me()), "\"owner_uid\":999999"),
            MintRefusal::WrongUser,
        ),
        ("not json".to_owned(), MintRefusal::Malformed),
        (
            good("power_profile_hold").replace("w19-power", ""),
            MintRefusal::Malformed,
        ),
    ];
    for (body, expected) in cases {
        let path = receipt(&root, &body);
        assert_eq!(
            mint(&root, &path, CampaignScope::PowerProfileHold).unwrap_err(),
            expected,
            "{body}"
        );
    }
    assert!(
        !root.join("journal.jsonl").exists(),
        "a refusal wrote a journal line"
    );
    assert_eq!(
        mint(
            &root,
            &root.join("campaign/absent.json"),
            CampaignScope::PowerProfileHold
        )
        .unwrap_err(),
        MintRefusal::Unreadable
    );
}

#[test]
fn a_receipt_anyone_could_have_written_is_not_trusted() {
    let root = tree("untrusted");
    let path = receipt(&root, &good("power_profile_hold"));
    // Group-writable file, group-writable directory, a symlink in the chain, a foreign owner.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();
    assert!(matches!(
        mint(&root, &path, CampaignScope::PowerProfileHold),
        Err(MintRefusal::UntrustedPath(_))
    ));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(root.join("campaign"), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        mint(&root, &path, CampaignScope::PowerProfileHold),
        Err(MintRefusal::UntrustedPath(_))
    ));
    fs::set_permissions(root.join("campaign"), fs::Permissions::from_mode(0o755)).unwrap();

    symlink(root.join("campaign"), root.join("link")).unwrap();
    assert!(matches!(
        mint(
            &root,
            &root.join("link/receipt.json"),
            CampaignScope::PowerProfileHold
        ),
        Err(MintRefusal::UntrustedPath(_))
    ));
    // The trusted user is someone else: this caller's own file proves nothing.
    let result = mint_with(
        &path,
        &root.join("journal.jsonl"),
        CampaignScope::PowerProfileHold,
        &HostFacts {
            now_unix: NOW,
            boot_id: BOOT,
            caller_uid: me(),
            anchor: &root,
            trusted_uid: me().wrapping_add(1),
        },
    );
    assert!(matches!(result, Err(MintRefusal::UntrustedPath(_))));
    // A path outside the trust anchor is never walked.
    assert!(matches!(
        mint(
            &root,
            Path::new("/etc/hostname"),
            CampaignScope::PowerProfileHold
        ),
        Err(MintRefusal::UntrustedPath(_))
    ));
}

#[test]
fn nothing_is_issued_when_the_journal_cannot_be_written() {
    let root = tree("journal");
    let path = receipt(&root, &good("power_profile_hold"));
    let result = mint_with(
        &path,
        &root.join("no-such-dir/journal.jsonl"),
        CampaignScope::PowerProfileHold,
        &HostFacts {
            now_unix: NOW,
            boot_id: BOOT,
            caller_uid: me(),
            anchor: &root,
            trusted_uid: me(),
        },
    );
    assert_eq!(result.unwrap_err(), MintRefusal::JournalFailed);
}

#[test]
fn the_production_factory_trusts_only_root_so_a_user_written_receipt_is_refused() {
    let root = tree("production");
    let path = receipt(&root, &good("power_profile_hold"));
    // The real factory walks from `/` and requires root ownership of every component.
    let result = mint_authorization(
        &path,
        &root.join("journal.jsonl"),
        CampaignScope::PowerProfileHold,
    );
    assert!(
        matches!(result, Err(MintRefusal::UntrustedPath(_))) || me() == 0,
        "{result:?}"
    );
}
