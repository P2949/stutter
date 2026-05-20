//! Profile TOML rendering and template generation.
//!
//! Owns deterministic profile TOML serialization and topology template text. Does not own parsing,
//! validation, matching, or action application.

use super::Profile;
use crate::process_tree::TaskClass;

pub fn render_profiles_toml(profiles: &[Profile]) -> String {
    let mut out = String::new();

    for profile in profiles {
        out.push_str("[[profile]]\n");
        out.push_str("name = ");
        out.push_str(&toml_quoted_string(&profile.name));
        out.push_str("\n\n");

        for rule in &profile.rules {
            out.push_str("[[profile.rules]]\n");
            if let Some(affinity) = &rule.affinity {
                out.push_str("affinity = ");
                out.push_str(&toml_quoted_string(&affinity.to_range_string()));
                out.push('\n');
            }

            if let Some(nice) = rule.nice {
                out.push_str("nice = ");
                out.push_str(&nice.to_string());
                out.push('\n');
            }

            if let Some(ionice) = rule.ionice {
                out.push_str("ionice = ");
                out.push_str(&toml_quoted_string(&ionice.label()));
                out.push('\n');
            }

            if !rule.match_class.is_empty() {
                out.push_str("match_class = [");
                for (idx, class) in rule.match_class.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&toml_quoted_string(task_class_toml_name(*class)));
                }
                out.push_str("]\n");
            }

            if !rule.match_comm.is_empty() {
                out.push_str("match_comm = [");
                for (idx, pattern) in rule.match_comm.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&toml_quoted_string(pattern.raw()));
                }
                out.push_str("]\n");
            }

            out.push('\n');
        }
    }

    out
}

fn toml_quoted_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');

    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }

    quoted.push('"');
    quoted
}

fn task_class_toml_name(class: TaskClass) -> &'static str {
    match class {
        TaskClass::Game => "Game",
        TaskClass::GameRenderThread => "GameRenderThread",
        TaskClass::GameWorkerThread => "GameWorkerThread",
        TaskClass::GameHelper => "GameHelper",
        TaskClass::Launcher => "Launcher",
        TaskClass::WineServer => "WineServer",
        TaskClass::GameScope => "GameScope",
        TaskClass::Compositor => "Compositor",
        TaskClass::AudioRealtime => "AudioRealtime",
        TaskClass::Input => "Input",
        TaskClass::BrowserForeground => "BrowserForeground",
        TaskClass::BrowserBackground => "BrowserBackground",
        TaskClass::BrowserRenderer => "BrowserRenderer",
        TaskClass::BrowserGpu => "BrowserGpu",
        TaskClass::BrowserNetwork => "BrowserNetwork",
        TaskClass::Compiler => "Compiler",
        TaskClass::Linker => "Linker",
        TaskClass::Indexer => "Indexer",
        TaskClass::PackageManager => "PackageManager",
        TaskClass::BuildJob => "BuildJob",
        TaskClass::StorageDaemon => "StorageDaemon",
        TaskClass::NetworkDaemon => "NetworkDaemon",
        TaskClass::KernelThread => "KernelThread",
        TaskClass::IrqThread => "IrqThread",
        TaskClass::Editor => "Editor",
        TaskClass::Terminal => "Terminal",
        TaskClass::Shell => "Shell",
        TaskClass::Media => "Media",
        TaskClass::Recorder => "Recorder",
        TaskClass::VirtualMachine => "VirtualMachine",
        TaskClass::SteamRuntime => "SteamRuntime",
        TaskClass::Render => "Render",
        TaskClass::Helper => "Helper",
        TaskClass::Service => "Service",
        TaskClass::Unknown => "Unknown",
    }
}

pub fn generate_topology_template() -> String {
    let mut out = String::new();
    out.push_str("[[profile]]\n");
    out.push_str("name = \"baseline-online\"\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"online\"\n");
    out.push_str("match_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\", \"GameScope\", \"Compositor\", \"AudioRealtime\", \"Input\", \"BrowserForeground\", \"BrowserBackground\", \"BrowserRenderer\", \"BrowserGpu\", \"BrowserNetwork\", \"Compiler\", \"Linker\", \"Indexer\", \"PackageManager\", \"BuildJob\", \"StorageDaemon\", \"NetworkDaemon\", \"KernelThread\", \"IrqThread\", \"Editor\", \"Terminal\", \"Shell\", \"Media\", \"Recorder\", \"VirtualMachine\", \"SteamRuntime\", \"Helper\", \"Service\", \"Unknown\"]\n\n");
    out.push_str("[[profile]]\n");
    out.push_str("name = \"game-main-suggested\"\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"<edit-me>\"\n");
    out.push_str("match_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\"]\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"<edit-me>\"\n");
    out.push_str("match_class = [\"GameScope\", \"Compositor\"]\n");
    out.push_str("\n[[profile.rules]]\n");
    out.push_str("nice = 10\n");
    out.push_str("ionice = \"idle\"\n");
    out.push_str("match_class = [\"BuildJob\", \"Indexer\", \"PackageManager\"]\n");
    out
}
