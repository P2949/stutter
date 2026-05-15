use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IrqLine {
    pub irq: String,
    pub counts_by_cpu: Vec<u64>,
    pub total: u64,
    pub kind: String,
    pub name: String,
    pub raw: String,
}

pub fn run_inspect_irqs(json: bool, filters: &[String], top: usize) -> Result<()> {
    let contents =
        std::fs::read_to_string("/proc/interrupts").context("failed to read /proc/interrupts")?;

    let lines = parse_proc_interrupts(&contents)?;

    let filtered = filter_sort_and_limit_irqs(lines, filters, top);

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
    } else {
        if filtered.is_empty() {
            if !filters.is_empty() {
                println!("No IRQ lines matched the requested filters.");
            } else {
                println!("No IRQ lines found in /proc/interrupts.");
            }
        } else {
            print!("{}", render_irqs_human(&filtered));
        }
    }

    Ok(())
}

pub fn parse_proc_interrupts(input: &str) -> Result<Vec<IrqLine>> {
    let mut lines = input.lines();

    let header = lines
        .find(|line| !line.trim().is_empty())
        .context("missing /proc/interrupts header")?;

    let cpu_count = header
        .split_whitespace()
        .filter(|token| token.starts_with("CPU"))
        .count();

    if cpu_count == 0 {
        anyhow::bail!("could not detect CPU columns in /proc/interrupts");
    }

    let mut parsed = Vec::new();

    for raw_line in lines {
        let raw = raw_line.to_string();
        let line = raw_line.trim();

        if line.is_empty() {
            continue;
        }

        let Some((irq_part, rest)) = line.split_once(':') else {
            continue;
        };

        let irq = irq_part.trim().to_string();

        if irq.is_empty() {
            continue;
        }

        let mut tokens = rest.split_whitespace();

        let mut counts_by_cpu = Vec::new();

        for _ in 0..cpu_count {
            let Some(token) = tokens.next() else {
                break;
            };

            match token.parse::<u64>() {
                Ok(value) => counts_by_cpu.push(value),
                Err(_) => break,
            }
        }

        if counts_by_cpu.is_empty() {
            continue;
        }

        let total = counts_by_cpu.iter().copied().sum();

        let remaining: Vec<&str> = tokens.collect();

        let kind = remaining.first().copied().unwrap_or_default().to_string();

        let name = if remaining.len() > 1 {
            remaining[1..].join(" ")
        } else {
            String::new()
        };

        parsed.push(IrqLine {
            irq,
            counts_by_cpu,
            total,
            kind,
            name,
            raw,
        });
    }

    Ok(parsed)
}

pub fn is_numeric_irq(irq: &str) -> bool {
    irq.parse::<u32>().is_ok()
}

fn matches_filters(line: &IrqLine, filters: &[String]) -> bool {
    if filters.is_empty() {
        return true;
    }

    let haystack = format!("{} {} {} {}", line.irq, line.kind, line.name, line.raw).to_lowercase();

    filters
        .iter()
        .all(|filter| haystack.contains(&filter.to_lowercase()))
}

pub fn filter_sort_and_limit_irqs(
    mut lines: Vec<IrqLine>,
    filters: &[String],
    top: usize,
) -> Vec<IrqLine> {
    lines.retain(|line| matches_filters(line, filters));

    lines.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.irq.cmp(&b.irq)));

    lines.truncate(top);
    lines
}

pub fn render_irqs_human(lines: &[IrqLine]) -> String {
    let mut out = String::new();

    out.push_str("IRQ        total        kind       name\n");

    for line in lines {
        out.push_str(&format!(
            "{:<10} {:<12} {:<10} {}\n",
            line.irq, line.total, line.kind, line.name
        ));
    }

    let numeric: Vec<&IrqLine> = lines
        .iter()
        .filter(|line| is_numeric_irq(&line.irq))
        .take(5)
        .collect();

    if !numeric.is_empty() {
        out.push_str("\nSuggestions:\n");

        for line in numeric {
            out.push_str(&format!(
                "  Use: stutter monitor --irq-latency --irq {}\n",
                line.irq
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROC_INTERRUPTS_FIXTURE: &str = r#"
           CPU0       CPU1       CPU2       CPU3
  0:         12          0          0          0   IO-APIC   2-edge      timer
146:    1000000    2000000    3000000    4000000   PCI-MSI 524288-edge  amdgpu
147:       1000       2000       3000       4000   PCI-MSI 524289-edge  xhci_hcd
NMI:        100        200        300        400   Non-maskable interrupts
LOC:      11111      22222      33333      44444   Local timer interrupts
"#;

    #[test]
    fn parses_numeric_irq_and_total() {
        let lines = parse_proc_interrupts(PROC_INTERRUPTS_FIXTURE).unwrap();

        let amdgpu = lines.iter().find(|line| line.irq == "146").unwrap();

        assert_eq!(
            amdgpu.counts_by_cpu,
            vec![1_000_000, 2_000_000, 3_000_000, 4_000_000]
        );
        assert_eq!(amdgpu.total, 10_000_000);
        assert!(amdgpu.name.contains("amdgpu"));
    }

    #[test]
    fn parses_non_numeric_irq_without_suggesting_as_numeric() {
        let lines = parse_proc_interrupts(PROC_INTERRUPTS_FIXTURE).unwrap();

        let nmi = lines.iter().find(|line| line.irq == "NMI").unwrap();

        assert_eq!(nmi.total, 1000);
        assert!(!is_numeric_irq(&nmi.irq));
    }

    #[test]
    fn filter_matches_amdgpu() {
        let lines = parse_proc_interrupts(PROC_INTERRUPTS_FIXTURE).unwrap();
        let filtered = filter_sort_and_limit_irqs(lines, &[String::from("amdgpu")], 30);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].irq, "146");
    }

    #[test]
    fn irq_lines_serialize_to_json() {
        let lines = parse_proc_interrupts(PROC_INTERRUPTS_FIXTURE).unwrap();

        let json = serde_json::to_string(&lines).unwrap();

        assert!(json.contains("\"irq\":\"146\""));
        assert!(json.contains("\"total\":10000000"));
    }

    #[test]
    fn human_output_suggests_only_numeric_irqs() {
        let lines = parse_proc_interrupts(PROC_INTERRUPTS_FIXTURE).unwrap();
        let filtered = filter_sort_and_limit_irqs(lines, &[], 30);
        let output = render_irqs_human(&filtered);

        assert!(output.contains("stutter monitor --irq-latency --irq 146"));
        assert!(!output.contains("stutter monitor --irq-latency --irq NMI"));
        assert!(!output.contains("stutter monitor --irq-latency --irq LOC"));
    }
}
