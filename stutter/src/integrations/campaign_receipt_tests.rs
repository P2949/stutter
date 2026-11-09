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

fn minted(root: &Path, scope: CampaignScope) -> (PathBuf, OfflineAuthorization) {
    let path = receipt(
        root,
        &good("power_profile_hold").replace("power_profile_hold", scope.name()),
    );
    let authorization = mint(root, &path, scope).unwrap();
    (path, authorization)
}

fn facts(now: u64, boot: &str, uid: u32) -> UseFacts {
    UseFacts {
        now_unix: now,
        boot_id: boot.to_owned(),
        caller_uid: uid,
    }
}

#[test]
fn a_minted_authorization_is_bound_to_its_scope_and_cannot_be_used_for_another() {
    let root = tree("scope");
    let (_, auth) = minted(&root, CampaignScope::PowerProfileHold);
    let now = facts(NOW, BOOT, me());
    assert_eq!(
        auth.require_at(CampaignScope::PowerProfileHold, &now),
        Ok(())
    );
    for other in [
        CampaignScope::SchedulerSwitch,
        CampaignScope::UnitProperties,
    ] {
        assert_eq!(
            auth.require_at(other, &now),
            Err(AuthorizationDenied::WrongScope {
                required: other,
                granted: CampaignScope::PowerProfileHold
            })
        );
        assert!(auth.require_for_restore_at(other, &now).is_err());
    }
    // No scope can be asked of a receipt that does not grant it, so it never mints at all.
    let path = receipt(&root, &good("power_profile_hold"));
    assert_eq!(
        mint(&root, &path, CampaignScope::UnitProperties).unwrap_err(),
        MintRefusal::ScopeNotGranted
    );
}

#[test]
fn expiry_boot_and_user_are_rechecked_when_the_authorization_is_used() {
    let root = tree("use-time");
    let (_, auth) = minted(&root, CampaignScope::SchedulerSwitch);
    let scope = CampaignScope::SchedulerSwitch;
    assert_eq!(auth.require_at(scope, &facts(NOW + 59, BOOT, me())), Ok(()));
    assert_eq!(
        auth.require_at(scope, &facts(NOW + 60, BOOT, me())),
        Err(AuthorizationDenied::Expired)
    );
    assert_eq!(
        auth.require_at(scope, &facts(NOW, "another-boot", me())),
        Err(AuthorizationDenied::WrongBoot)
    );
    assert_eq!(
        auth.require_at(scope, &facts(NOW, BOOT, me().wrapping_add(1))),
        Err(AuthorizationDenied::WrongUser)
    );
    // Putting state back is never blocked by an expiry or a boot change, only by scope and user.
    assert_eq!(
        auth.require_for_restore_at(scope, &facts(NOW + 10_000, "another-boot", me())),
        Ok(())
    );
    assert_eq!(
        auth.require_for_restore_at(scope, &facts(NOW, BOOT, me().wrapping_add(1))),
        Err(AuthorizationDenied::WrongUser)
    );
}

#[test]
fn removing_or_replacing_the_receipt_revokes_the_authorization() {
    let root = tree("revoke");
    let (path, auth) = minted(&root, CampaignScope::PowerProfileHold);
    let now = facts(NOW, BOOT, me());
    assert_eq!(
        auth.require_at(CampaignScope::PowerProfileHold, &now),
        Ok(())
    );
    // Replaced by another file with the same content: a different file, so not the same grant.
    let replacement = root.join("campaign/replacement.json");
    fs::copy(&path, &replacement).unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o644)).unwrap();
    fs::rename(&replacement, &path).unwrap();
    assert_eq!(
        auth.require_at(CampaignScope::PowerProfileHold, &now),
        Err(AuthorizationDenied::Revoked)
    );
    let (path, auth) = minted(&root, CampaignScope::PowerProfileHold);
    fs::remove_file(&path).unwrap();
    assert_eq!(
        auth.require_at(CampaignScope::PowerProfileHold, &now),
        Err(AuthorizationDenied::Revoked)
    );
    // Revocation does not stop a restore: cleanup stays possible.
    assert_eq!(
        auth.require_for_restore_at(CampaignScope::PowerProfileHold, &now),
        Ok(())
    );
}

#[test]
fn a_symlinked_receipt_and_a_symlinked_journal_are_refused_not_followed() {
    let root = tree("nofollow");
    let real = receipt(&root, &good("power_profile_hold"));
    let link = root.join("campaign/link.json");
    symlink(&real, &link).unwrap();
    assert!(matches!(
        mint(&root, &link, CampaignScope::PowerProfileHold),
        Err(MintRefusal::UntrustedPath(_))
    ));
    let target = root.join("victim.txt");
    fs::write(&target, "keep").unwrap();
    symlink(&target, root.join("journal.jsonl")).unwrap();
    assert_eq!(
        mint(&root, &real, CampaignScope::PowerProfileHold).unwrap_err(),
        MintRefusal::JournalFailed
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
}
