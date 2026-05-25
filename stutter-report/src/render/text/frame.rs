use crate::model::FrameDiagnosis;
use super::diagnosis::render_diagnosis_detail_lines;
use super::pushln;

pub fn render_frame_diagnosis(rank: usize, diag: &FrameDiagnosis) -> String {
    let mut output = String::new();
    pushln(
        &mut output,
        format!(
            "{}. elapsed={}ms frametime={:.1}ms diagnosis: {}",
            rank,
            diag.frame_elapsed_ms,
            diag.frametime_ms,
            diag.diagnosis.report_summary()
        ),
    );
    output.push_str(&render_diagnosis_detail_lines(&diag.diagnosis, "  "));
    output.trim_end().to_owned()
}
