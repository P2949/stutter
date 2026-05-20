//! Process/task classification heuristics.
//!
//! Owns built-in task classification and process-name predicate helpers. Does not own procfs
//! reading, target expansion, snapshots, tree rendering, or community-rule database loading.

use super::model::TaskClass;

/// Classification priority (highest to lowest):
/// 1. GameScope: The compositor running the game.
/// 2. Compositor: System-level window managers and compositors.
/// 3. WineServer: The core Wine/Proton server process.
/// 4. SteamRuntime: Container and runtime tools (pressure-vessel, bwrap, etc).
/// 5. Launcher: Game-specific or platform launchers (Epic, EA, Ubisoft, etc).
/// 6. Helper: Known system or background helpers (svchost, steamwebhelper, etc).
/// 7. Game: Likely game process based on path patterns (steamapps/common, etc).
/// 8. GameHelper: Any other .exe process not caught by the above.
pub fn classify_task(comm: &str, process_comm: &str, cmdline: &str) -> TaskClass {
    classify_task_with_context(comm, process_comm, cmdline, "", "", None)
}

pub fn classify_task_with_context(
    comm: &str,
    process_comm: &str,
    cmdline: &str,
    exe_path: &str,
    cgroup_path: &str,
    sched_policy: Option<u32>,
) -> TaskClass {
    let lower_comm = comm.to_ascii_lowercase();
    let lower_process_comm = process_comm.to_ascii_lowercase();
    let lower_cmdline = cmdline.to_ascii_lowercase();
    let lower_exe_path = exe_path.to_ascii_lowercase();
    let lower_cgroup_path = cgroup_path.to_ascii_lowercase();

    let combined = [
        lower_comm.as_str(),
        lower_process_comm.as_str(),
        lower_cmdline.as_str(),
        lower_exe_path.as_str(),
        lower_cgroup_path.as_str(),
    ]
    .join(" ");

    // 1. Critical System Threads (Kernel, Input, IRQ)
    if is_bracketed_kernel_comm(&lower_comm) {
        return TaskClass::KernelThread;
    }
    if is_input_thread(&lower_comm, &combined) {
        return TaskClass::Input;
    }
    if is_irq_thread(&lower_comm, &combined) {
        return TaskClass::IrqThread;
    }

    // 2. Audio (Realtime)
    if is_audio_realtime_comm(&lower_comm)
        || is_audio_realtime_comm(&lower_process_comm)
        || contains_any(
            &combined,
            &[
                "pipewire",
                "wireplumber",
                "pulseaudio",
                "jackd",
                "easyeffects",
            ],
        )
        || (matches!(sched_policy, Some(1 | 2 | 6)) && is_audio_looking_process(&combined))
    {
        return TaskClass::AudioRealtime;
    }

    // 3. Infrastructure (Compositors, WineServer, Steam Services)
    if lower_comm == "gamescope" || lower_process_comm == "gamescope" {
        return TaskClass::GameScope;
    }
    if lower_comm == "wineserver" || lower_process_comm == "wineserver" {
        return TaskClass::WineServer;
    }
    if contains_any(
        &lower_comm,
        &[
            "sway",
            "kwin",
            "mutter",
            "gnome-shell",
            "weston",
            "hyprland",
        ],
    ) {
        return TaskClass::Compositor;
    }
    if lower_comm == "steam"
        || lower_process_comm == "steam"
        || lower_comm == "steamwebhelper"
        || lower_process_comm == "steamwebhelper"
    {
        return TaskClass::Service;
    }

    // 4. Specialized Threads in Game/Browser (Must come before generic process match)
    let is_game = is_game_exe(&lower_exe_path)
        || is_game_comm(&lower_process_comm)
        || is_game_cgroup(&lower_cgroup_path);

    if is_game {
        if is_game_render_comm(&lower_comm) {
            return TaskClass::GameRenderThread;
        }
        if lower_comm.contains("worker")
            || lower_comm.contains("task")
            || lower_comm.contains("job")
        {
            return TaskClass::GameWorkerThread;
        }
    }

    if is_browser_process(&lower_comm, &lower_process_comm, &combined) {
        if contains_any(&combined, &["gpu process", "--type=gpu-process"])
            || lower_comm.contains("gpu process")
        {
            return TaskClass::BrowserGpu;
        }
        if contains_any(
            &combined,
            &[
                "utility process",
                "--type=utility",
                "network service",
                "socket process",
            ],
        ) {
            return TaskClass::BrowserNetwork;
        }
        if contains_any(
            &combined,
            &[
                "web content",
                "isolated web co",
                "rdd process",
                "--type=renderer",
                "renderer",
            ],
        ) {
            return TaskClass::BrowserRenderer;
        }
        if contains_any(
            &combined,
            &[
                "--background",
                "background",
                "crashpad",
                "updater",
                "extension process",
            ],
        ) {
            return TaskClass::BrowserBackground;
        }
        return TaskClass::BrowserForeground;
    }

    // 5. Development & System Work
    if is_indexer_comm(&lower_comm) {
        return TaskClass::Indexer;
    }
    if is_compiler_comm(&lower_comm) {
        return TaskClass::Compiler;
    }
    if is_linker_comm(&lower_comm) {
        return TaskClass::Linker;
    }
    if is_package_manager_comm(&lower_comm) || lower_cmdline.contains(" emerge ") {
        return TaskClass::PackageManager;
    }
    if is_build_job_comm(&lower_comm) {
        return TaskClass::BuildJob;
    }

    // 6. Daemons & Services
    if is_storage_daemon_comm(&lower_comm) {
        return TaskClass::StorageDaemon;
    }
    if is_network_daemon_comm(&lower_comm) {
        return TaskClass::NetworkDaemon;
    }

    // 7. Community app-name hints, then generic process fallbacks.
    #[cfg(test)]
    if !is_service_looking_process(&lower_process_comm, &lower_cgroup_path)
        && let Some(hit) = crate::community_rules::classify_process_identity(
            &crate::community_rules::CommunityProcessIdentity {
                thread_comm: comm,
                process_comm,
                cmdline,
                exe_path,
                cgroup_path,
            },
        )
    {
        return hit.class;
    }

    if is_game {
        return TaskClass::Game;
    }
    if is_steam_runtime_comm(&lower_process_comm) || lower_exe_path.contains("pressure-vessel") {
        return TaskClass::SteamRuntime;
    }
    if is_launcher_comm(&lower_comm) || is_launcher_comm(&lower_process_comm) {
        return TaskClass::Launcher;
    }
    if is_service_looking_process(&lower_process_comm, &lower_cgroup_path)
        || lower_process_comm.contains("helper")
    {
        return TaskClass::Service;
    }

    // 8. Other App Categories
    if is_editor_comm(&lower_comm) {
        return TaskClass::Editor;
    }
    if is_terminal_comm(&lower_comm) {
        return TaskClass::Terminal;
    }
    if is_shell_comm(&lower_comm) {
        return TaskClass::Shell;
    }
    if is_media_comm(&lower_comm) {
        return TaskClass::Media;
    }
    if is_recorder_comm(&lower_comm) {
        return TaskClass::Recorder;
    }
    if is_vm_comm(&lower_comm) {
        return TaskClass::VirtualMachine;
    }

    if lower_exe_path.ends_with(".exe") {
        return TaskClass::GameHelper;
    }

    TaskClass::Unknown
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn is_bracketed_kernel_comm(comm: &str) -> bool {
    comm.starts_with('[') && comm.ends_with(']')
}

fn is_irq_thread(comm: &str, combined: &str) -> bool {
    comm.starts_with("irq/")
        || comm.starts_with("irq-")
        || combined.contains(" irq/")
        || combined.contains(" irq-")
}

fn is_input_thread(comm: &str, combined: &str) -> bool {
    comm.contains("input") || combined.contains("libinput")
}

fn is_audio_realtime_comm(comm: &str) -> bool {
    matches!(
        comm,
        "pipewire" | "wireplumber" | "pulseaudio" | "jackd" | "easyeffects"
    )
}

fn is_audio_looking_process(combined: &str) -> bool {
    contains_any(
        combined,
        &[
            "audio",
            "alsa",
            "jack",
            "pipewire",
            "pulseaudio",
            "pulse",
            "easyeffects",
        ],
    )
}

fn is_game_render_comm(comm: &str) -> bool {
    contains_any(comm, &["render", "rhi", "dxvk", "vulkan", "gpu"])
}

fn is_game_exe(exe_path: &str) -> bool {
    exe_path.contains("steamapps/common")
        || exe_path.contains("/games/")
        || exe_path.contains("pressure-vessel")
}

fn is_game_comm(comm: &str) -> bool {
    contains_any(comm, &["steam", "proton", "wine"])
}

fn is_game_cgroup(cgroup: &str) -> bool {
    cgroup.contains("steam") || cgroup.contains("games")
}

fn is_browser_process(comm: &str, process_comm: &str, combined: &str) -> bool {
    let names = ["firefox", "chrome", "chromium", "brave", "browser"];
    contains_any(comm, &names)
        || contains_any(process_comm, &names)
        || contains_any(combined, &["--type=renderer", "--type=gpu-process"])
}

fn is_compiler_comm(comm: &str) -> bool {
    contains_any(comm, &["rustc", "gcc", "g++", "clang", "cc1", "cc1plus"])
}

fn is_linker_comm(comm: &str) -> bool {
    matches!(comm, "ld" | "ld.lld" | "ld.gold" | "mold" | "gold" | "lld")
}

fn is_indexer_comm(comm: &str) -> bool {
    contains_any(comm, &["clangd", "rust-analyzer", "ccls", "indexer"])
}

fn is_package_manager_comm(comm: &str) -> bool {
    contains_any(comm, &["emerge", "portage", "pacman", "apt", "dnf"])
}

fn is_build_job_comm(comm: &str) -> bool {
    contains_any(comm, &["cargo", "make", "ninja", "cmake", "meson"])
}

fn is_storage_daemon_comm(comm: &str) -> bool {
    contains_any(comm, &["udisks", "jbd2", "btrfs", "zfs", "io_uring"])
}

fn is_network_daemon_comm(comm: &str) -> bool {
    contains_any(
        comm,
        &[
            "networkmanager",
            "systemd-network",
            "dhcpcd",
            "wpa_supplicant",
        ],
    )
}

fn is_steam_runtime_comm(comm: &str) -> bool {
    contains_any(comm, &["pressure-vessel", "bwrap"])
}

fn is_launcher_comm(comm: &str) -> bool {
    contains_any(
        comm,
        &["epicgames", "origin", "uplay", "battle.net", "lutris"],
    )
}

fn is_service_looking_process(comm: &str, cgroup: &str) -> bool {
    comm == "systemd"
        || cgroup.contains(".service")
        || cgroup.contains("/system.slice/")
        || comm.ends_with("d")
}

fn is_editor_comm(comm: &str) -> bool {
    contains_any(comm, &["code", "vscodium", "kate", "nvim", "vim", "emacs"])
}

fn is_terminal_comm(comm: &str) -> bool {
    contains_any(
        comm,
        &[
            "alacritty",
            "kitty",
            "wezterm",
            "foot",
            "gnome-terminal",
            "konsole",
        ],
    )
}

fn is_shell_comm(comm: &str) -> bool {
    matches!(comm, "bash" | "zsh" | "fish" | "sh")
}

fn is_media_comm(comm: &str) -> bool {
    contains_any(comm, &["vlc", "mpv", "spotify"])
}

fn is_recorder_comm(comm: &str) -> bool {
    contains_any(comm, &["obs", "gpu-screen-recorder", "recorder"])
}

fn is_vm_comm(comm: &str) -> bool {
    contains_any(comm, &["qemu", "virt", "virtualbox", "vmware"])
}

pub(crate) fn contains_likely_game_cmdline(cmdline: &str) -> bool {
    if !cmdline.contains(".exe") {
        return false;
    }

    if !cmdline.contains("steamapps/common")
        && !cmdline.contains("\\steamapps\\common")
        && !cmdline.contains("/games/")
        && !cmdline.contains("\\games\\")
    {
        return false;
    }

    !contains_known_non_game_exe(cmdline)
}

fn contains_known_non_game_exe(text: &str) -> bool {
    [
        "steam.exe",
        "steamwebhelper.exe",
        "steamerrorreporter.exe",
        "xalia.exe",
        "explorer.exe",
        "services.exe",
        "winedevice.exe",
        "svchost.exe",
        "plugplay.exe",
        "rpcss.exe",
        "tabtip.exe",
        "rundll32.exe",
        "wineboot.exe",
        "winemenubuilder.exe",
        "conhost.exe",
        "regsvr32.exe",
        "msiexec.exe",
    ]
    .iter()
    .any(|name| text.contains(name))
}
