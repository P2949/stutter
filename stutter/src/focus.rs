#![allow(dead_code)]
use serde::{Deserialize, Serialize};

const SCHED_FIFO: u32 = 1;
const SCHED_RR: u32 = 2;
const SCHED_DEADLINE: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum SystemTaskClass {
    Game,
    GameRenderThread,
    GameWorkerThread,
    WineServer,
    GameScope,
    Compositor,

    AudioRealtime,
    Input,

    BrowserForeground,
    BrowserBackground,
    BrowserRenderer,
    BrowserGpu,
    BrowserNetwork,

    BuildJob,
    Compiler,
    Linker,
    Indexer,
    PackageManager,

    StorageDaemon,
    NetworkDaemon,
    KernelThread,
    IrqThread,

    Editor,
    Terminal,
    Shell,
    Media,
    Recorder,
    VirtualMachine,

    Service,

    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PriorityBand {
    CriticalRealtime,
    ForegroundLatency,
    Interactive,
    Throughput,
    Background,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub class: SystemTaskClass,
    pub priority_band: PriorityBand,
    pub confidence: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProcessIdentity<'a> {
    pub pid: u32,
    pub ppid: u32,
    pub comm: &'a str,
    pub cmdline: &'a str,
    pub exe_path: Option<&'a str>,
    pub cgroup_path: Option<&'a str>,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ThreadIdentity<'a> {
    pub tid: u32,
    pub process_pid: u32,
    pub process_class: SystemTaskClass,
    pub thread_comm: &'a str,
    pub process_comm: &'a str,
    pub sched_policy: Option<u32>,
}

pub fn classify_process(identity: &ProcessIdentity<'_>) -> Classification {
    let comm = identity.comm.to_ascii_lowercase();
    let cmdline = identity.cmdline.to_ascii_lowercase();
    let exe_path = identity.exe_path.unwrap_or_default().to_ascii_lowercase();
    let cgroup_path = identity
        .cgroup_path
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut reasons = Vec::new();

    let (class, confidence) = if comm.starts_with("irq/")
        || comm.starts_with("irq-")
        || comm.contains("irq")
    {
        reasons.push(format!("comm '{}' looks like an IRQ thread", identity.comm));
        (SystemTaskClass::IrqThread, 0.95)
    } else if identity.ppid == 2 || comm.starts_with("kworker") || comm.starts_with("ksoftirqd") {
        reasons.push(format!(
            "pid={} ppid={} comm='{}' looks like a kernel thread",
            identity.pid, identity.ppid, identity.comm
        ));
        (SystemTaskClass::KernelThread, 0.9)
    } else if comm == "wineserver" {
        reasons.push("comm is exactly 'wineserver'".to_owned());
        (SystemTaskClass::WineServer, 0.98)
    } else if comm == "gamescope"
        || exe_path.contains("/gamescope")
        || cgroup_path.contains("gamescope")
    {
        reasons.push("process identity matches gamescope".to_owned());
        (SystemTaskClass::GameScope, 0.95)
    } else if comm.contains("pipewire")
        || comm.contains("wireplumber")
        || comm.contains("pulseaudio")
        || comm.contains("jackd")
    {
        reasons.push(format!("comm '{}' matches an audio service", identity.comm));
        (SystemTaskClass::AudioRealtime, 0.9)
    } else if comm.contains("sway")
        || comm.contains("kwin")
        || comm.contains("mutter")
        || comm.contains("gnome-shell")
        || comm.contains("weston")
        || comm.contains("hyprland")
    {
        reasons.push(format!("comm '{}' matches a compositor", identity.comm));
        (SystemTaskClass::Compositor, 0.9)
    } else if comm.contains("firefox")
        || comm.contains("chrome")
        || comm.contains("chromium")
        || comm.contains("brave")
        || comm.contains("browser")
        || exe_path.contains("/firefox")
        || exe_path.contains("/chrome")
        || exe_path.contains("/chromium")
    {
        reasons.push("process identity matches a browser family".to_owned());
        if cmdline.contains("--type=gpu-process") {
            reasons.push("cmdline contains '--type=gpu-process'".to_owned());
            (SystemTaskClass::BrowserGpu, 0.9)
        } else if cmdline.contains("--type=renderer") || cmdline.contains("web content") {
            reasons.push("cmdline indicates a browser renderer child".to_owned());
            (SystemTaskClass::BrowserRenderer, 0.85)
        } else if cmdline.contains("--type=utility") && cmdline.contains("network") {
            reasons.push("cmdline indicates a browser network utility child".to_owned());
            (SystemTaskClass::BrowserNetwork, 0.85)
        } else if cgroup_path.contains("background") {
            reasons.push("cgroup path contains 'background'".to_owned());
            (SystemTaskClass::BrowserBackground, 0.75)
        } else {
            reasons.push("browser process role is foreground or parent by default".to_owned());
            (SystemTaskClass::BrowserForeground, 0.7)
        }
    } else if comm == "clang"
        || comm == "clang++"
        || comm == "gcc"
        || comm == "g++"
        || comm == "rustc"
        || comm == "cc1"
        || comm == "cc1plus"
    {
        reasons.push(format!("comm '{}' matches a compiler", identity.comm));
        (SystemTaskClass::Compiler, 0.95)
    } else if comm == "ld" || comm == "ld.lld" || comm == "mold" || comm == "gold" || comm == "lld"
    {
        reasons.push(format!("comm '{}' matches a linker", identity.comm));
        (SystemTaskClass::Linker, 0.95)
    } else if comm == "clangd"
        || comm.contains("rust-analyzer")
        || comm.contains("ccls")
        || comm.contains("indexer")
    {
        reasons.push(format!(
            "comm '{}' matches an indexer/language server",
            identity.comm
        ));
        (SystemTaskClass::Indexer, 0.9)
    } else if comm == "cargo"
        || comm == "make"
        || comm == "ninja"
        || comm == "cmake"
        || comm == "meson"
        || cmdline.contains(" emerge ")
        || comm == "emerge"
        || comm == "portage"
        || comm == "pacman"
        || comm == "apt"
        || comm == "dnf"
    {
        reasons.push(format!(
            "comm '{}' or cmdline matches build/package work",
            identity.comm
        ));
        if comm == "emerge"
            || comm == "portage"
            || comm == "pacman"
            || comm == "apt"
            || comm == "dnf"
        {
            (SystemTaskClass::PackageManager, 0.9)
        } else {
            (SystemTaskClass::BuildJob, 0.85)
        }
    } else if comm.contains("udisks")
        || comm.contains("jbd2")
        || comm.contains("btrfs")
        || comm.contains("zfs")
        || comm.contains("io_uring")
    {
        reasons.push(format!(
            "comm '{}' matches storage daemon/kernel storage work",
            identity.comm
        ));
        (SystemTaskClass::StorageDaemon, 0.8)
    } else if comm.contains("networkmanager")
        || comm.contains("systemd-network")
        || comm.contains("dhcpcd")
        || comm.contains("wpa_supplicant")
    {
        reasons.push(format!(
            "comm '{}' matches network daemon work",
            identity.comm
        ));
        (SystemTaskClass::NetworkDaemon, 0.85)
    } else if comm.contains("code")
        || comm.contains("vscodium")
        || comm.contains("kate")
        || comm.contains("nvim")
        || comm.contains("vim")
        || comm.contains("emacs")
    {
        reasons.push(format!("comm '{}' matches an editor", identity.comm));
        (SystemTaskClass::Editor, 0.8)
    } else if comm.contains("alacritty")
        || comm.contains("kitty")
        || comm.contains("wezterm")
        || comm.contains("foot")
        || comm.contains("gnome-terminal")
        || comm.contains("konsole")
    {
        reasons.push(format!("comm '{}' matches a terminal", identity.comm));
        (SystemTaskClass::Terminal, 0.85)
    } else if comm == "bash" || comm == "zsh" || comm == "fish" || comm == "sh" {
        reasons.push(format!("comm '{}' matches a shell", identity.comm));
        (SystemTaskClass::Shell, 0.9)
    } else if comm.contains("vlc")
        || comm.contains("mpv")
        || comm.contains("spotify")
        || comm.contains("pipewire-media-session")
    {
        reasons.push(format!("comm '{}' matches media playback", identity.comm));
        (SystemTaskClass::Media, 0.8)
    } else if comm.contains("obs")
        || comm.contains("gpu-screen-recorder")
        || comm.contains("recorder")
    {
        reasons.push(format!(
            "comm '{}' matches recording software",
            identity.comm
        ));
        (SystemTaskClass::Recorder, 0.85)
    } else if comm.contains("qemu")
        || comm.contains("virt")
        || comm.contains("virtualbox")
        || comm.contains("vmware")
    {
        reasons.push(format!(
            "comm '{}' matches virtual machine software",
            identity.comm
        ));
        (SystemTaskClass::VirtualMachine, 0.85)
    } else if cgroup_path.contains("steam")
        || cgroup_path.contains("games")
        || cmdline.contains("steamapps")
        || cmdline.contains(".exe")
        || exe_path.contains("steamapps")
    {
        reasons.push("cgroup, cmdline, or exe path suggests a game process".to_owned());
        (SystemTaskClass::Game, 0.75)
    } else if comm == "steam" || comm.contains("electron") || comm == "python" {
        reasons.push(format!(
            "comm '{}' is ambiguous and needs more evidence before assigning a specific class",
            identity.comm
        ));
        (SystemTaskClass::Unknown, 0.35)
    } else if identity.pid == 1
        || cgroup_path.contains(".service")
        || cgroup_path.contains("/system.slice/")
    {
        reasons.push("pid/cgroup suggests a generic service".to_owned());
        (SystemTaskClass::Service, 0.6)
    } else {
        reasons.push("no strong process classification rule matched".to_owned());
        (SystemTaskClass::Unknown, 0.0)
    };

    Classification {
        class,
        priority_band: priority_band_for_class(class, identity.sched_policy),
        confidence,
        reasons,
    }
}

pub fn classify_thread(identity: &ThreadIdentity<'_>) -> Classification {
    let thread_comm = identity.thread_comm.to_ascii_lowercase();
    let process_comm = identity.process_comm.to_ascii_lowercase();
    let mut reasons = Vec::new();

    let (class, confidence) = if thread_comm.starts_with("irq/")
        || thread_comm.starts_with("irq-")
        || thread_comm.contains("irq")
    {
        reasons.push(format!(
            "thread_comm '{}' looks like an IRQ thread",
            identity.thread_comm
        ));
        (SystemTaskClass::IrqThread, 0.95)
    } else if thread_comm.contains("audio")
        || thread_comm.contains("pipewire")
        || thread_comm.contains("jack")
        || is_realtime_policy(identity.sched_policy)
    {
        reasons.push(format!(
            "thread_comm '{}' or sched_policy {:?} suggests realtime/audio work",
            identity.thread_comm, identity.sched_policy
        ));
        (SystemTaskClass::AudioRealtime, 0.85)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::Game | SystemTaskClass::GameScope
    ) && (thread_comm.contains("render")
        || thread_comm.contains("rhi")
        || thread_comm.contains("dxvk")
        || thread_comm.contains("vulkan")
        || thread_comm.contains("gpu"))
    {
        reasons.push(format!(
            "game process thread_comm '{}' suggests render/GPU work",
            identity.thread_comm
        ));
        (SystemTaskClass::GameRenderThread, 0.85)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::Game | SystemTaskClass::GameScope
    ) && (thread_comm.contains("worker")
        || thread_comm.contains("task")
        || thread_comm.contains("job")
        || thread_comm.contains("pool"))
    {
        reasons.push(format!(
            "game process thread_comm '{}' suggests worker/job work",
            identity.thread_comm
        ));
        (SystemTaskClass::GameWorkerThread, 0.8)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserBackground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
    ) && (thread_comm.contains("compositor") || thread_comm.contains("render"))
    {
        reasons.push(format!(
            "browser process thread_comm '{}' suggests renderer work",
            identity.thread_comm
        ));
        (SystemTaskClass::BrowserRenderer, 0.75)
    } else if matches!(identity.process_class, SystemTaskClass::BrowserForeground)
        && (thread_comm.contains("socket")
            || thread_comm.contains("network")
            || thread_comm.contains("dns"))
    {
        reasons.push(format!(
            "browser process thread_comm '{}' suggests network work",
            identity.thread_comm
        ));
        (SystemTaskClass::BrowserNetwork, 0.75)
    } else if identity.process_class != SystemTaskClass::Unknown {
        reasons.push(format!(
            "thread inherits process_class {:?} from process_comm '{}'",
            identity.process_class, identity.process_comm
        ));
        (identity.process_class, 0.6)
    } else if process_comm == "python"
        || process_comm.contains("electron")
        || process_comm == "steam"
    {
        reasons.push(format!(
            "process_comm '{}' is ambiguous and thread_comm '{}' did not disambiguate it",
            identity.process_comm, identity.thread_comm
        ));
        (SystemTaskClass::Unknown, 0.3)
    } else {
        reasons.push("no strong thread classification rule matched".to_owned());
        (SystemTaskClass::Unknown, 0.0)
    };

    Classification {
        class,
        priority_band: priority_band_for_class(class, identity.sched_policy),
        confidence,
        reasons,
    }
}

pub fn priority_band_for_class(class: SystemTaskClass, sched_policy: Option<u32>) -> PriorityBand {
    if is_realtime_policy(sched_policy) {
        return PriorityBand::CriticalRealtime;
    }

    match class {
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input | SystemTaskClass::IrqThread => {
            PriorityBand::CriticalRealtime
        }
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameScope
        | SystemTaskClass::Compositor
        | SystemTaskClass::BrowserForeground => PriorityBand::ForegroundLatency,
        SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Media
        | SystemTaskClass::Recorder
        | SystemTaskClass::VirtualMachine => PriorityBand::Interactive,
        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::PackageManager => PriorityBand::Throughput,
        SystemTaskClass::BrowserBackground
        | SystemTaskClass::BrowserNetwork
        | SystemTaskClass::Indexer
        | SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::Service => PriorityBand::Background,
        SystemTaskClass::Unknown => PriorityBand::Unknown,
    }
}

pub fn legacy_task_class_for_system_class(
    class: SystemTaskClass,
) -> crate::process_tree::TaskClass {
    use crate::process_tree::TaskClass;

    match class {
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread => TaskClass::Game,

        SystemTaskClass::WineServer => TaskClass::WineServer,
        SystemTaskClass::GameScope => TaskClass::GameScope,
        SystemTaskClass::Compositor => TaskClass::Compositor,

        SystemTaskClass::Service
        | SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread => TaskClass::Service,

        _ => TaskClass::Helper,
    }
}

fn is_realtime_policy(sched_policy: Option<u32>) -> bool {
    matches!(sched_policy, Some(SCHED_FIFO | SCHED_RR | SCHED_DEADLINE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_tree::TaskClass;

    #[test]
    fn legacy_task_class_maps_game_related_system_classes_to_game() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Game),
            TaskClass::Game
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameRenderThread),
            TaskClass::Game
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameWorkerThread),
            TaskClass::Game
        );
    }

    #[test]
    fn legacy_task_class_preserves_special_foreground_classes() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::WineServer),
            TaskClass::WineServer
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameScope),
            TaskClass::GameScope
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Compositor),
            TaskClass::Compositor
        );
    }

    #[test]
    fn legacy_task_class_maps_daemon_and_kernel_classes_to_service() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Service),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::StorageDaemon),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::NetworkDaemon),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::KernelThread),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::IrqThread),
            TaskClass::Service
        );
    }

    #[test]
    fn legacy_task_class_maps_all_other_system_classes_to_helper() {
        let classes = [
            SystemTaskClass::AudioRealtime,
            SystemTaskClass::Input,
            SystemTaskClass::BrowserForeground,
            SystemTaskClass::BrowserBackground,
            SystemTaskClass::BrowserRenderer,
            SystemTaskClass::BrowserGpu,
            SystemTaskClass::BrowserNetwork,
            SystemTaskClass::BuildJob,
            SystemTaskClass::Compiler,
            SystemTaskClass::Linker,
            SystemTaskClass::Indexer,
            SystemTaskClass::PackageManager,
            SystemTaskClass::Editor,
            SystemTaskClass::Terminal,
            SystemTaskClass::Shell,
            SystemTaskClass::Media,
            SystemTaskClass::Recorder,
            SystemTaskClass::VirtualMachine,
            SystemTaskClass::Unknown,
        ];

        for class in classes {
            assert_eq!(legacy_task_class_for_system_class(class), TaskClass::Helper);
        }
    }
}
