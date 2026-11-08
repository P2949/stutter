//! Deterministic fuzz-style robustness tests for every parser that reads untrusted text.
//!
//! The inputs are pseudo-random but fixed by seed, so a failure reproduces. The property is the
//! same for all of them: arbitrary bytes and text never panic, never grow state past its bound, and
//! never turn into an accepted request unless they are exactly a valid one.
use crate::{
    autotune::objective::source::{
        Aggregation, Direction, KpiContract, MetricClass, MetricSpec, SampleStore, SourceBinding,
    },
    boot_environment::{parse_isolation, preempt_mode},
    telemetry::registration::{MAX_REGISTRATIONS, PeerCredentials, Registry, Response},
    topology::parse_cpu_list,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// A string drawn from tokens that stress these parsers: separators, signs, huge numbers,
    /// control characters, non-ASCII and path-like pieces.
    fn text(&mut self, max_tokens: usize) -> String {
        const TOKENS: &[&str] = &[
            "register",
            "cancel",
            "inspect",
            "apply",
            " ",
            "  ",
            "\t",
            "\n",
            "\r",
            "\0",
            ",",
            "-",
            ":",
            "=",
            "0",
            "1",
            "42",
            "-1",
            "4294967296",
            "18446744073709551616",
            "1e999",
            "NaN",
            "inf",
            "..",
            "/",
            "../",
            "{",
            "}",
            "[",
            "]",
            "\"",
            "metric",
            "value",
            "monotonic_ns",
            "population",
            "nohz_full=",
            "isolcpus=",
            "rcu_nocbs=",
            "domain,managed_irq,",
            "0-3",
            "3-0",
            "a-b",
            "é",
            "日本",
            "\u{202e}",
            "💥",
            "x",
            "game-1",
        ];
        (0..self.below(max_tokens + 1))
            .map(|_| TOKENS[self.below(TOKENS.len())])
            .collect()
    }
}

#[test]
fn the_registration_protocol_never_panics_never_overflows_and_only_accepts_exact_requests() {
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    let mut registry = Registry::new(1000);
    let peer = PeerCredentials { pid: 7, uid: 1000 };
    for _ in 0..20_000 {
        let line = rng.text(8);
        let response = registry.handle(peer, &line);
        if matches!(response, Response::Registered) {
            // Only a well-formed `register <id> 7` is ever accepted.
            let words: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(words.len(), 3, "{line:?}");
            assert_eq!((words[0], words[2]), ("register", "7"), "{line:?}");
        }
    }
    // The bound held however many valid-looking requests arrived.
    let Response::Inspected(all) = registry.handle(peer, "inspect") else {
        panic!("inspect must answer");
    };
    assert!(all.len() <= MAX_REGISTRATIONS);
    // A peer of another user can never do anything.
    let stranger = PeerCredentials { pid: 7, uid: 0 };
    for _ in 0..2_000 {
        assert_eq!(
            registry.handle(stranger, &rng.text(6)),
            Response::Denied("wrong_user")
        );
    }
}

fn kpi_contract() -> KpiContract {
    let spec = |id: &str| MetricSpec {
        id: id.to_owned(),
        unit: "ms".to_owned(),
        class: MetricClass::Latency,
        direction: Direction::LowerIsBetter,
        aggregation: Aggregation::Mean,
        max_age_ms: 5_000,
        min_population: 1,
        desired_value: None,
        meaningful_effect_percent: 10.0,
        equivalent_within_percent: 3.0,
    };
    KpiContract {
        binding: SourceBinding {
            uid: 1000,
            pid: 42,
            start_ticks: 7,
        },
        primary: spec("request_p99"),
        hard_guardrails: vec![spec("error_rate")],
        advisory_guardrails: vec![],
    }
}

#[test]
fn the_kpi_ingest_never_panics_on_arbitrary_text_and_bytes() {
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    let mut store = SampleStore::new(kpi_contract());
    let peer = PeerCredentials { pid: 42, uid: 1000 };
    let mut accepted = 0;
    for _ in 0..20_000 {
        // Half pure noise, half structurally JSON-like with hostile field values.
        let line = if rng.below(2) == 0 {
            rng.text(12)
        } else {
            format!(
                r#"{{"metric":"{}","value":{},"monotonic_ns":{},"work_unit":null,"population":{},"kind":"{}"}}"#,
                ["request_p99", "error_rate", "other", ""][rng.below(4)],
                ["1.5", "-3", "1e999", "null", "\"x\"", "0"][rng.below(6)],
                [
                    "1000000000",
                    "0",
                    "-1",
                    "99999999999999999999",
                    "5000000000"
                ][rng.below(5)],
                ["1", "0", "-2", "3"][rng.below(4)],
                ["summary", "raw_event", "bogus"][rng.below(3)],
            )
        };
        if store.ingest_line(peer, 7, &line, 2_000_000_000).is_ok() {
            accepted += 1;
        }
    }
    // Some hostile-looking samples are valid ones; the point is that nothing panicked and the
    // store did not accept thousands of copies of the same timestamp.
    assert!(accepted < 20_000);
    // Wrong identity is refused whatever the line says.
    for _ in 0..1_000 {
        let line = rng.text(8);
        assert!(
            store
                .ingest_line(PeerCredentials { pid: 43, uid: 1000 }, 7, &line, 0)
                .is_err()
        );
        assert!(store.ingest_line(peer, 8, &line, 0).is_err());
    }
}

#[test]
fn cpu_lists_and_command_line_parameters_never_panic_and_stay_bounded() {
    let mut rng = Rng(0xFEED_FACE_0BAD_F00D);
    for _ in 0..20_000 {
        let text = rng.text(10);
        if let Ok(list) = parse_cpu_list(&text) {
            // A parsed list is finite and ascending ranges only: no gigantic expansion from a
            // short string.
            assert!(list.len() <= 1 << 20, "{text:?} expanded to {}", list.len());
        }
        let isolation = parse_isolation(&text);
        assert!(isolation.nohz_full.len() <= 1 << 20);
        let _ = preempt_mode(&text);
    }
}

#[test]
fn a_short_cpu_list_cannot_expand_into_billions_of_ids() {
    // Unbounded, this one string would allocate about sixteen gibibytes.
    for hostile in ["0-4294967295", "0-4294967294", "5-65536", "0-99999999"] {
        assert!(parse_cpu_list(hostile).is_err(), "{hostile}");
    }
    assert_eq!(parse_cpu_list("0-65535").unwrap().len(), 65_536);
    assert_eq!(parse_cpu_list("0-3,8").unwrap(), vec![0, 1, 2, 3, 8]);
    assert!(
        parse_isolation("nohz_full=0-4294967295")
            .nohz_full
            .is_empty()
    );
}
