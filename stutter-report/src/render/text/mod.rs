pub mod cluster;
pub mod correlation;
pub mod diagnosis;
pub mod frame;
pub mod header;
pub mod quality;

pub(crate) fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

pub fn render_report(model: &crate::model::ReportModel) -> String {
    let mut output = String::new();

    if let Some(header) = &model.header {
        output.push_str(&header::render_header(header));
    }

    if let Some(data_quality) = &model.data_quality {
        output.push_str(&quality::render_data_quality(data_quality));
    }

    if !model.clusters.is_empty() {
        pushln(&mut output, "spike clusters");
        pushln(&mut output, "--------------");
        for (i, cluster) in model.clusters.iter().enumerate() {
            pushln(&mut output, cluster::render_cluster(i + 1, cluster));
        }
        pushln(&mut output, "");
    }

    if !model.frames.is_empty() {
        pushln(&mut output, "frame diagnosis");
        pushln(&mut output, "---------------");
        for (i, frame) in model.frames.iter().enumerate() {
            pushln(&mut output, frame::render_frame_diagnosis(i + 1, frame));
        }
        pushln(&mut output, "");
    }

    if let Some(correlations) = &model.correlations {
        pushln(&mut output, "correlations");
        pushln(&mut output, "------------");
        correlation::render_correlation_sections(&mut output, correlations);
        pushln(&mut output, "");
    }

    output.trim_end().to_string() + "\n"
}
