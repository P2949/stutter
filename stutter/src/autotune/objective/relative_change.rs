/// The symmetric relative-change verdict between a baseline and a candidate value where lower is
/// better. Compile throughput (elapsed time) and frame pacing (frame p99) both decide with it, and
/// the calibration campaigns apply this exact function, so each rule exists once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RelativeChange {
    Improved(f64),
    Regressed(f64),
    Inconclusive(f64),
}

pub fn classify_relative_change(
    baseline: f64,
    candidate: f64,
    threshold_percent: f64,
) -> RelativeChange {
    let change_percent = (candidate - baseline) / baseline * 100.0;
    if change_percent >= threshold_percent {
        RelativeChange::Regressed(change_percent)
    } else if -change_percent >= threshold_percent {
        RelativeChange::Improved(-change_percent)
    } else {
        RelativeChange::Inconclusive(change_percent)
    }
}
