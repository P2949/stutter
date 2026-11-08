use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::{elf::parse_build_id, *};

fn input(path: &str, sha: &str) -> InputRef {
    InputRef {
        path: path.into(),
        sha256: sha.into(),
    }
}

fn plan(kind: Kind) -> Plan {
    Plan {
        kind,
        source_revision: "a".repeat(40),
        compiler: "clang".into(),
        compiler_version: "19.1".into(),
        flags: vec!["-O2".into()],
        training_inputs: vec![input("train/a", "1")],
        test_inputs: vec![input("test/a", "2")],
        input_binary: PathBuf::from("/work/app"),
        output_binary: PathBuf::from("/work/app.optimized"),
        profile: Some(ProfileRef {
            path: "app.prof".into(),
            sha256: "3".into(),
            build_id: "abcd".into(),
        }),
        guardrails: Guardrails {
            max_startup_regression_percent: 5.0,
            max_size_growth_percent: 20.0,
            max_build_time_ratio: 3.0,
        },
    }
}

fn tools(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

fn same_build(_: &Path) -> Option<String> {
    Some("abcd".into())
}

#[test]
fn a_complete_plan_with_its_tools_has_no_findings() {
    assert!(
        validate(
            &plan(Kind::Pgo),
            &tools(&["clang", "llvm-profdata"]),
            same_build
        )
        .is_empty()
    );
    assert!(
        validate(
            &plan(Kind::Bolt),
            &tools(&["perf", "perf2bolt", "llvm-bolt"]),
            same_build
        )
        .is_empty()
    );
}

#[test]
fn training_must_not_overlap_the_test_inputs_and_neither_may_be_empty() {
    let mut leaky = plan(Kind::Pgo);
    leaky.test_inputs = vec![input("test/copy-of-train", "1")];
    let findings = validate(&leaky, &tools(&["clang", "llvm-profdata"]), same_build);
    assert_eq!(
        findings,
        vec![Finding::TrainingOverlapsTest("test/copy-of-train".into())]
    );
    let mut empty = plan(Kind::Pgo);
    empty.training_inputs.clear();
    empty.test_inputs.clear();
    let findings = validate(&empty, &tools(&["clang", "llvm-profdata"]), same_build);
    assert!(
        findings.contains(&Finding::EmptyInputs("training"))
            && findings.contains(&Finding::EmptyInputs("test"))
    );
}

#[test]
fn the_output_is_a_separate_file_and_never_an_installed_path() {
    let mut same = plan(Kind::Pgo);
    same.output_binary = same.input_binary.clone();
    assert!(
        validate(&same, &tools(&["clang", "llvm-profdata"]), same_build)
            .contains(&Finding::OutputIsInput)
    );
    for installed in [
        "/nix/store/x-app/bin/app",
        "/usr/bin/app",
        "/usr/local/bin/app",
        "/run/current-system/sw/bin/app",
    ] {
        let mut hot = plan(Kind::Pgo);
        hot.output_binary = PathBuf::from(installed);
        assert!(
            validate(&hot, &tools(&["clang", "llvm-profdata"]), same_build)
                .iter()
                .any(|f| matches!(f, Finding::OutputIsInstalledPath(_))),
            "{installed}"
        );
    }
}

#[test]
fn a_profile_from_a_different_build_or_an_unreadable_build_id_is_refused() {
    let all = tools(&["perf", "perf2bolt", "llvm-bolt"]);
    let other = |_: &Path| Some("ffff".to_owned());
    assert_eq!(
        validate(&plan(Kind::Bolt), &all, other),
        vec![Finding::BuildIdMismatch {
            profile: "abcd".into(),
            binary: "ffff".into()
        }]
    );
    assert_eq!(
        validate(&plan(Kind::Bolt), &all, |_| None),
        vec![Finding::BuildIdUnreadable]
    );
    let mut none = plan(Kind::Bolt);
    none.profile = None;
    assert_eq!(
        validate(&none, &all, same_build),
        vec![Finding::ProfileRequired]
    );
}

#[test]
fn a_missing_tool_is_unavailable_never_a_pass_and_bad_guardrails_are_refused() {
    let findings = validate(&plan(Kind::AutoFdo), &tools(&["clang"]), same_build);
    assert!(findings.contains(&Finding::Unavailable("perf")));
    assert!(findings.contains(&Finding::Unavailable("create_llvm_prof")));
    let mut bad = plan(Kind::Pgo);
    bad.guardrails.max_size_growth_percent = f64::NAN;
    assert!(
        validate(&bad, &tools(&["clang", "llvm-profdata"]), same_build)
            .contains(&Finding::BadGuardrails)
    );
    assert_eq!(
        required_tools(Kind::Pgo, "gcc-13"),
        vec!["gcc", "llvm-profdata"]
    );
}

/// A minimal little-endian ELF64 with one SHT_NOTE section holding a GNU build id.
fn synthetic_elf(build_id: &[u8]) -> Vec<u8> {
    let mut note = Vec::new();
    note.extend(4u32.to_le_bytes());
    note.extend((build_id.len() as u32).to_le_bytes());
    note.extend(3u32.to_le_bytes());
    note.extend(b"GNU\0");
    note.extend(build_id);
    while note.len() % 4 != 0 {
        note.push(0);
    }
    let note_offset = 64usize;
    let section_offset = note_offset + note.len();
    let mut file = vec![0u8; 64];
    file[0..4].copy_from_slice(b"\x7fELF");
    file[4] = 2;
    file[5] = 1;
    file[0x28..0x30].copy_from_slice(&(section_offset as u64).to_le_bytes());
    file[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
    file[0x3c..0x3e].copy_from_slice(&1u16.to_le_bytes());
    file.extend(&note);
    let mut header = vec![0u8; 64];
    header[4..8].copy_from_slice(&7u32.to_le_bytes());
    header[0x18..0x20].copy_from_slice(&(note_offset as u64).to_le_bytes());
    header[0x20..0x28].copy_from_slice(&(note.len() as u64).to_le_bytes());
    file.extend(header);
    file
}

#[test]
fn the_build_id_is_read_from_the_note_section_and_garbage_is_refused() {
    let elf = synthetic_elf(&[0xde, 0xad, 0xbe, 0xef, 0x01, 0x02]);
    assert_eq!(parse_build_id(&elf).as_deref(), Some("deadbeef0102"));
    assert_eq!(parse_build_id(b"not an elf"), None);
    assert_eq!(
        parse_build_id(&elf[..70]),
        None,
        "a truncated file is not read past its end"
    );
    let mut big_endian = elf.clone();
    big_endian[5] = 2;
    assert_eq!(parse_build_id(&big_endian), None);
}
