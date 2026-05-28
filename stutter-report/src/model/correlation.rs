use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextReportCorrelationSections {
    pub sections: Vec<TextReportCorrelationSection>,
}

impl TextReportCorrelationSections {
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    pub fn push_section(&mut self, section: TextReportCorrelationSection) {
        self.sections.push(section);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextReportCorrelationSection {
    pub title: String,
    pub lines: Vec<String>,
}

impl TextReportCorrelationSection {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            lines: Vec::new(),
        }
    }

    pub fn push_line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
}
