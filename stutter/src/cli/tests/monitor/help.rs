use clap::CommandFactory;

use super::Cli;

#[test]
fn monitor_help_documents_irq_latency_dependency() {
    let help = render_monitor_help();

    assert!(
        help.contains("--irq-latency"),
        "monitor help should list --irq-latency flag:\n{help}"
    );
    assert!(
        help.contains("requires at least one explicit --irq"),
        "monitor help should explain that --irq-latency requires --irq:\n{help}"
    );
    assert!(
        help.contains("--irq <IRQ>"),
        "monitor help should list --irq <IRQ> argument:\n{help}"
    );
    assert!(
        help.contains("repeat for multiple IRQs"),
        "monitor help should explain repeated --irq usage:\n{help}"
    );
}

fn render_monitor_help() -> String {
    let mut command = Cli::command();

    let monitor = command
        .find_subcommand_mut("monitor")
        .expect("monitor subcommand should exist");

    let mut output = Vec::new();
    monitor
        .write_help(&mut output)
        .expect("clap can render monitor help");

    String::from_utf8(output).expect("clap help should be valid UTF-8")
}
