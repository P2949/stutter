use crate::model::TextReportCorrelationSections;
use super::pushln;

pub fn render_correlation_sections(
    output: &mut String,
    correlations: &TextReportCorrelationSections,
) {
    for section in &correlations.sections {
        pushln(output, &section.title);
        pushln(output, "-".repeat(section.title.len()));
        for line in &section.lines {
            pushln(output, line);
        }
        pushln(output, "");
    }
}
