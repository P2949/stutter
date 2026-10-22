//! `StutterScore` versus controlled scheduler degradation (0, 1, 2, 4, 8, 16 ms). A SCHED_FIFO
//! contender holds the target's CPU for N ms of every 20 ms (requires root). Evaluation is
//! the pure `score_evaluation` module over production score and runnable-latency code.

use super::{
    evidence::LiveAcceptanceSuite,
    game_runnable::{Contention, install_game_binary, run_scenario},
    *,
};
use crate::score_evaluation;

const LEVELS: [(&str, u64); 6] = [
    ("level_00ms", 0),
    ("level_01ms", 1),
    ("level_02ms", 2),
    ("level_04ms", 4),
    ("level_08ms", 8),
    ("level_16ms", 16),
];

pub(super) fn run(root: &Path, out: &Path, binary: &Path, seconds: u64) -> anyhow::Result<()> {
    ensure!(cfg!(target_os = "linux"), "live acceptance requires Linux");
    fs::create_dir(out).context("acceptance output must be a new directory")?;
    let out = fs::canonicalize(out)?;
    let binary = root.join(binary);
    fs::write(
        out.join("acceptance.toml"),
        "# Isolated score-degradation defaults.\n",
    )?;
    let mut stage = "preflight";
    let mut monotonic = true;
    let result = (|| -> anyhow::Result<Value> {
        evidence::metadata(root, &out, &binary, LiveAcceptanceSuite::ScoreDegradation)?;
        command(
            &binary,
            &args(&["doctor", "--json"]),
            &out,
            "doctor",
            Duration::from_secs(15),
        )?;
        let doctor: Value = serde_json::from_slice(&fs::read(out.join("doctor.stdout"))?)?;
        ensure!(
            doctor["overall"] == "Pass" || doctor["overall"] == "Warn",
            "doctor preflight failed"
        );
        command(
            Path::new("chrt"),
            &args(&["--version"]),
            &out,
            "chrt",
            Duration::from_secs(5),
        )
        .context("chrt is required for the SCHED_FIFO contender")?;
        let cpu = allowed_cpu()?;
        let game = install_game_binary(&out)?;
        stage = "levels";
        let mut observations = Vec::new();
        let mut per_level = Vec::new();
        for (name, ms) in LEVELS {
            let (scenario, value) = run_scenario(
                &binary,
                &game,
                &out,
                name,
                cpu,
                seconds,
                Contention::Burst(ms),
            )?;
            observations.push(score_evaluation::observe(
                ms as f64,
                &scenario.records,
                scenario.pid,
                scenario.ticks,
            ));
            per_level.push(value);
        }
        stage = "evaluation";
        let evaluation = score_evaluation::evaluate(observations)?;
        monotonic = evaluation["monotonic_in_degradation"] == true;
        Ok(json!({"levels":per_level,"evaluation":evaluation}))
    })();
    let status = match (&result, monotonic) {
        (Ok(_), true) => "PASS",
        (Ok(_), false) => "INCONCLUSIVE",
        (Err(_), _) if stage == "preflight" => "BLOCKED",
        (Err(_), _) => "FAIL",
    };
    let error = result.as_ref().err().map(|e| format!("{e:#}")).or_else(|| {
        (!monotonic).then(|| {
            "score is not monotonic in controlled degradation; weights need review".to_owned()
        })
    });
    let cases = if status == "PASS" {
        json!([{"name":"score_vs_degradation","status":"PASS"}])
    } else {
        json!([])
    };
    let receipt = json!({"schema_version":1,"status":status,"stage":stage,"evidence_kind":"controlled_real","cases":cases,
        "measurements":result.as_ref().ok(),"error":error,
        "scope":"StutterScore and runnable latency versus a deterministic SCHED_FIFO CPU-hold contender at 0,1,2,4,8,16 ms on a disposable game-path workload",
        "not_claimed":["frame-time metrics (synthetic workload emits none)","real games","weights change (retained unless non-monotonic)","portable verified bundle until exported"]});
    evidence::write_result(&out, LiveAcceptanceSuite::ScoreDegradation, receipt)?;
    println!("{status}: score-degradation evidence {}", out.display());
    result.map(|_| ())
}
