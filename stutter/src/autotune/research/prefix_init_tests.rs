use std::os::unix::fs::PermissionsExt;

use super::*;

const NOW: u64 = 1_000_000;
const ARM: ProtonSource = ProtonSource::Ge;

struct Fixture {
    _root: tempfile::TempDir,
    journal: File,
    compat: std::path::PathBuf,
    settings: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let study = std::fs::canonicalize(root.path()).unwrap();
    std::fs::set_permissions(&study, std::fs::Permissions::from_mode(0o700)).unwrap();
    let compat = study.join("compatdata/ge");
    std::fs::create_dir_all(&compat).unwrap();
    let settings = compat.join("pfx/UserSettings.json");
    Fixture {
        journal: File::open(&study).unwrap(),
        _root: root,
        compat,
        settings,
    }
}

/// What a supervised launch leaves behind: a prefix with its registry and the game's settings.
fn initialize(f: &Fixture) {
    let pfx = f.compat.join("pfx");
    std::fs::create_dir_all(&pfx).unwrap();
    for name in ["system.reg", "user.reg", "userdef.reg"] {
        std::fs::write(pfx.join(name), b"x").unwrap();
    }
    std::fs::write(&f.settings, b"{}").unwrap();
    std::fs::write(f.compat.join("version"), b"11.0-100").unwrap();
}

fn uid() -> u32 {
    crate::syscall::geteuid()
}

fn walk(f: &Fixture, journal: &mut InitJournal, stages: &[InitStage]) {
    for stage in stages {
        journal
            .advance(*stage, &f.compat, &f.settings, NOW + step())
            .unwrap();
    }
}

/// Every record is a little later than the one before.
fn step() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CLOCK: AtomicU64 = AtomicU64::new(0);
    CLOCK.fetch_add(1, Ordering::SeqCst)
}

#[test]
fn only_a_cold_prefix_needs_initializing_and_a_half_made_one_is_not_initializable() {
    let f = fixture();
    let needs = |f: &Fixture| plan(&[(ARM, f.compat.as_path())]);
    assert_eq!(needs(&f), vec![(ARM, InitNeed::Initialize)]);
    // Partly created (a registry file but not all): neither cold nor warm.
    std::fs::create_dir_all(f.compat.join("pfx")).unwrap();
    std::fs::write(f.compat.join("pfx/system.reg"), b"x").unwrap();
    assert_eq!(needs(&f), vec![(ARM, InitNeed::Indeterminate)]);
    initialize(&f);
    assert_eq!(needs(&f), vec![(ARM, InitNeed::NotNeeded)]);
}

#[test]
fn a_cold_arm_walks_planned_launched_settled_verified_and_is_then_initialized() {
    let f = fixture();
    let mut journal = InitJournal::open(&f.journal, ARM, uid()).unwrap();
    assert!(journal.profile_allowed(), "no journal, nothing in flight");
    assert!(
        !initialized(&f.journal, ARM, &f.compat, uid()).unwrap(),
        "cold"
    );
    walk(&f, &mut journal, &[InitStage::Planned, InitStage::Launched]);
    assert!(
        !journal.profile_allowed(),
        "never while a launch is in flight"
    );
    initialize(&f);
    walk(&f, &mut journal, &[InitStage::Settled]);
    assert!(!journal.profile_allowed(), "never before it is verified");
    journal
        .advance(InitStage::Verified, &f.compat, &f.settings, NOW + 500)
        .unwrap();
    assert!(journal.verified() && journal.profile_allowed());
    // A fresh read of the journal says the same, and the arm is initialized.
    let reread = InitJournal::open(&f.journal, ARM, uid()).unwrap();
    assert_eq!(reread.stage(), Some(InitStage::Verified));
    assert!(initialized(&f.journal, ARM, &f.compat, uid()).unwrap());
}

#[test]
fn an_illegal_transition_a_late_settlement_and_a_verification_over_a_bare_prefix_are_refused() {
    let f = fixture();
    let mut journal = InitJournal::open(&f.journal, ARM, uid()).unwrap();
    // Nothing but planning can start a journal; stages cannot be skipped.
    assert!(
        journal
            .advance(InitStage::Launched, &f.compat, &f.settings, NOW)
            .is_err()
    );
    walk(&f, &mut journal, &[InitStage::Planned]);
    assert!(
        journal
            .advance(InitStage::Verified, &f.compat, &f.settings, NOW)
            .is_err()
    );
    walk(&f, &mut journal, &[InitStage::Launched, InitStage::Settled]);
    // Settled, but the prefix was never created: verification is refused.
    assert!(
        journal
            .advance(InitStage::Verified, &f.compat, &f.settings, NOW + 500)
            .is_err()
    );
    // A settlement after the bound is refused, and the launch is never extended.
    let g = fixture();
    let mut late = InitJournal::open(&g.journal, ARM, uid()).unwrap();
    walk(&g, &mut late, &[InitStage::Planned, InitStage::Launched]);
    assert!(
        late.advance(
            InitStage::Settled,
            &g.compat,
            &g.settings,
            NOW + 1_000 + MAX_INIT_SECONDS
        )
        .is_err()
    );
    // Giving up is always possible and ends the journal short of verified.
    late.advance(
        InitStage::Abandoned,
        &g.compat,
        &g.settings,
        NOW + 10_000 + MAX_INIT_SECONDS,
    )
    .unwrap();
    assert!(!late.verified() && !late.profile_allowed());
}

#[test]
fn a_forged_gapped_or_reordered_journal_is_not_a_chain() {
    let f = fixture();
    let mut journal = InitJournal::open(&f.journal, ARM, uid()).unwrap();
    walk(&f, &mut journal, &[InitStage::Planned, InitStage::Launched]);
    initialize(&f);
    walk(&f, &mut journal, &[InitStage::Settled]);
    journal
        .advance(InitStage::Verified, &f.compat, &f.settings, NOW + 500)
        .unwrap();
    let path = |n: u32| f._root.path().join(file_name(ARM, n));
    // Edit a middle record: the next record no longer names its digest.
    let text = std::fs::read_to_string(path(2)).unwrap();
    std::fs::write(path(2), text.replace("\"launched\"", "\"launched\" ")).unwrap();
    std::fs::set_permissions(path(2), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(InitJournal::open(&f.journal, ARM, uid()).is_err());
    assert!(initialized(&f.journal, ARM, &f.compat, uid()).is_err());
    // A gap: the second record removed with a later one present.
    let g = fixture();
    let mut j = InitJournal::open(&g.journal, ARM, uid()).unwrap();
    walk(
        &g,
        &mut j,
        &[
            InitStage::Planned,
            InitStage::Launched,
            InitStage::Abandoned,
        ],
    );
    std::fs::remove_file(g._root.path().join(file_name(ARM, 2))).unwrap();
    assert!(InitJournal::open(&g.journal, ARM, uid()).is_err());
}

#[test]
fn distant_gaps_and_unreadable_records_never_look_like_an_empty_journal() {
    let f = fixture();
    let distant = f._root.path().join(file_name(ARM, 9));
    std::fs::write(&distant, b"{}").unwrap();
    std::fs::set_permissions(&distant, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(InitJournal::open(&f.journal, ARM, uid()).is_err());
    std::fs::remove_file(&distant).unwrap();

    let first = f._root.path().join(file_name(ARM, 1));
    std::fs::write(&first, b"{}").unwrap();
    std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(InitJournal::open(&f.journal, ARM, uid()).is_err());
    std::fs::remove_file(&first).unwrap();
    std::os::unix::fs::symlink("missing", &first).unwrap();
    assert!(InitJournal::open(&f.journal, ARM, uid()).is_err());
}

#[test]
fn an_initialized_prefix_with_no_journal_is_the_users_own_and_a_verified_journal_over_a_lost_prefix_is_not()
 {
    let f = fixture();
    initialize(&f);
    // Warm and never ours to initialize: initialized.
    assert!(initialized(&f.journal, ARM, &f.compat, uid()).unwrap());
    // A journal that claims verification while the prefix has since vanished is not initialized.
    let g = fixture();
    let mut journal = InitJournal::open(&g.journal, ARM, uid()).unwrap();
    walk(&g, &mut journal, &[InitStage::Planned, InitStage::Launched]);
    initialize(&g);
    walk(&g, &mut journal, &[InitStage::Settled]);
    journal
        .advance(InitStage::Verified, &g.compat, &g.settings, NOW + 500)
        .unwrap();
    std::fs::remove_dir_all(g.compat.join("pfx")).unwrap();
    assert!(!initialized(&g.journal, ARM, &g.compat, uid()).unwrap());
}
