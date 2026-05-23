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
    let fields = AsciiFields {
        comm,
        process_comm,
        cmdline,
        exe_path,
        cgroup_path,
    };

    // 1. Critical System Threads (Kernel, Input, IRQ)
    if is_bracketed_kernel_comm(fields.comm) {
        return TaskClass::KernelThread;
    }
    if is_input_thread(fields.comm, &fields) {
        return TaskClass::Input;
    }
    if is_irq_thread(fields.comm, &fields) {
        return TaskClass::IrqThread;
    }

    // 2. Audio (Realtime)
    if is_audio_realtime_comm(fields.comm)
        || is_audio_realtime_comm(fields.process_comm)
        || contains_any_field(
            &fields,
            &[
                "pipewire",
                "wireplumber",
                "pulseaudio",
                "jackd",
                "easyeffects",
            ],
        )
        || (matches!(sched_policy, Some(1 | 2 | 6)) && is_audio_looking_process(&fields))
    {
        return TaskClass::AudioRealtime;
    }

    // 3. Infrastructure (Compositors, WineServer, Steam Services)
    if eq_ignore_ascii(fields.comm, "gamescope")
        || eq_ignore_ascii(fields.process_comm, "gamescope")
    {
        return TaskClass::GameScope;
    }
    if eq_ignore_ascii(fields.comm, "wineserver")
        || eq_ignore_ascii(fields.process_comm, "wineserver")
    {
        return TaskClass::WineServer;
    }
    if contains_any_ignore_ascii(
        fields.comm,
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
    if eq_ignore_ascii(fields.comm, "steam")
        || eq_ignore_ascii(fields.process_comm, "steam")
        || eq_ignore_ascii(fields.comm, "steamwebhelper")
        || eq_ignore_ascii(fields.process_comm, "steamwebhelper")
    {
        return TaskClass::Service;
    }

    // 4. Specialized Threads in Game/Browser (Must come before generic process match)
    let is_game = is_game_exe(fields.exe_path)
        || is_game_comm(fields.process_comm)
        || is_game_cgroup(fields.cgroup_path);

    if is_game {
        if is_game_render_comm(fields.comm) {
            return TaskClass::GameRenderThread;
        }
        if contains_ignore_ascii(fields.comm, "worker")
            || contains_ignore_ascii(fields.comm, "task")
            || contains_ignore_ascii(fields.comm, "job")
        {
            return TaskClass::GameWorkerThread;
        }
    }

    if is_browser_process(fields.comm, fields.process_comm, &fields) {
        if contains_any_field(&fields, &["gpu process", "--type=gpu-process"])
            || contains_ignore_ascii(fields.comm, "gpu process")
        {
            return TaskClass::BrowserGpu;
        }
        if contains_any_field(
            &fields,
            &[
                "utility process",
                "--type=utility",
                "network service",
                "socket process",
            ],
        ) {
            return TaskClass::BrowserNetwork;
        }
        if contains_any_field(
            &fields,
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
        if contains_any_field(
            &fields,
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
    if is_indexer_comm(fields.comm) {
        return TaskClass::Indexer;
    }
    if is_compiler_comm(fields.comm) {
        return TaskClass::Compiler;
    }
    if is_linker_comm(fields.comm) {
        return TaskClass::Linker;
    }
    if is_package_manager_comm(fields.comm) || contains_ignore_ascii(fields.cmdline, " emerge ") {
        return TaskClass::PackageManager;
    }
    if is_build_job_comm(fields.comm) {
        return TaskClass::BuildJob;
    }

    // 6. Daemons & Services
    if is_storage_daemon_comm(fields.comm) {
        return TaskClass::StorageDaemon;
    }
    if is_network_daemon_comm(fields.comm) {
        return TaskClass::NetworkDaemon;
    }

    // 7. Community app-name hints, then generic process fallbacks.
    #[cfg(test)]
    if !is_service_looking_process(fields.process_comm, fields.cgroup_path)
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
    if is_steam_runtime_comm(fields.process_comm)
        || contains_ignore_ascii(fields.exe_path, "pressure-vessel")
    {
        return TaskClass::SteamRuntime;
    }
    if is_launcher_comm(fields.comm) || is_launcher_comm(fields.process_comm) {
        return TaskClass::Launcher;
    }
    if is_service_looking_process(fields.process_comm, fields.cgroup_path)
        || contains_ignore_ascii(fields.process_comm, "helper")
    {
        return TaskClass::Service;
    }

    // 8. Other App Categories
    if is_editor_comm(fields.comm) {
        return TaskClass::Editor;
    }
    if is_terminal_comm(fields.comm) {
        return TaskClass::Terminal;
    }
    if is_shell_comm(fields.comm) {
        return TaskClass::Shell;
    }
    if is_media_comm(fields.comm) {
        return TaskClass::Media;
    }
    if is_recorder_comm(fields.comm) {
        return TaskClass::Recorder;
    }
    if is_vm_comm(fields.comm) {
        return TaskClass::VirtualMachine;
    }

    if ends_with_ignore_ascii(fields.exe_path, ".exe") {
        return TaskClass::GameHelper;
    }

    TaskClass::Unknown
}

#[derive(Clone, Copy)]
struct AsciiFields<'a> {
    comm: &'a str,
    process_comm: &'a str,
    cmdline: &'a str,
    exe_path: &'a str,
    cgroup_path: &'a str,
}

impl AsciiFields<'_> {
    fn contains_any(self, needle: &str) -> bool {
        contains_ignore_ascii(self.comm, needle)
            || contains_ignore_ascii(self.process_comm, needle)
            || contains_ignore_ascii(self.cmdline, needle)
            || contains_ignore_ascii(self.exe_path, needle)
            || contains_ignore_ascii(self.cgroup_path, needle)
    }
}

fn eq_ignore_ascii(value: &str, expected: &str) -> bool {
    value.eq_ignore_ascii_case(expected)
}

fn contains_any_ignore_ascii(haystack: &str, needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| contains_ignore_ascii(haystack, needle))
}

fn field_contains(fields: &AsciiFields<'_>, needle: &str) -> bool {
    fields.contains_any(needle)
}

fn contains_any_field(fields: &AsciiFields<'_>, needles: &[&str]) -> bool {
    needles.iter().any(|needle| field_contains(fields, needle))
}

fn contains_ignore_ascii(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }

    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| eq_ignore_ascii_bytes(window, needle))
}

fn ends_with_ignore_ascii(value: &str, suffix: &str) -> bool {
    let value = value.as_bytes();
    let suffix = suffix.as_bytes();
    if suffix.len() > value.len() {
        return false;
    }

    eq_ignore_ascii_bytes(&value[value.len() - suffix.len()..], suffix)
}

fn eq_ignore_ascii_bytes(left: &[u8], right: &[u8]) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn is_bracketed_kernel_comm(comm: &str) -> bool {
    comm.starts_with('[') && comm.ends_with(']')
}

fn is_irq_thread(comm: &str, fields: &AsciiFields<'_>) -> bool {
    starts_with_ignore_ascii(comm, "irq/")
        || starts_with_ignore_ascii(comm, "irq-")
        || field_contains(fields, " irq/")
        || field_contains(fields, " irq-")
}

fn is_input_thread(comm: &str, fields: &AsciiFields<'_>) -> bool {
    contains_ignore_ascii(comm, "input") || field_contains(fields, "libinput")
}

fn is_audio_realtime_comm(comm: &str) -> bool {
    eq_any_ignore_ascii(
        comm,
        &[
            "pipewire",
            "wireplumber",
            "pulseaudio",
            "jackd",
            "easyeffects",
        ],
    )
}

fn is_audio_looking_process(fields: &AsciiFields<'_>) -> bool {
    contains_any_field(
        fields,
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
    contains_any_ignore_ascii(comm, &["render", "rhi", "dxvk", "vulkan", "gpu"])
}

fn is_game_exe(exe_path: &str) -> bool {
    contains_ignore_ascii(exe_path, "steamapps/common")
        || contains_ignore_ascii(exe_path, "/games/")
        || contains_ignore_ascii(exe_path, "pressure-vessel")
}

fn is_game_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["steam", "proton", "wine"])
}

fn is_game_cgroup(cgroup: &str) -> bool {
    contains_ignore_ascii(cgroup, "steam") || contains_ignore_ascii(cgroup, "games")
}

fn is_browser_process(comm: &str, process_comm: &str, fields: &AsciiFields<'_>) -> bool {
    let names = ["firefox", "chrome", "chromium", "brave", "browser"];
    contains_any_ignore_ascii(comm, &names)
        || contains_any_ignore_ascii(process_comm, &names)
        || contains_any_field(fields, &["--type=renderer", "--type=gpu-process"])
}

fn is_compiler_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["rustc", "gcc", "g++", "clang", "cc1", "cc1plus"])
}

fn is_linker_comm(comm: &str) -> bool {
    eq_any_ignore_ascii(comm, &["ld", "ld.lld", "ld.gold", "mold", "gold", "lld"])
}

fn is_indexer_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["clangd", "rust-analyzer", "ccls", "indexer"])
}

fn is_package_manager_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["emerge", "portage", "pacman", "apt", "dnf"])
}

fn is_build_job_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["cargo", "make", "ninja", "cmake", "meson"])
}

fn is_storage_daemon_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["udisks", "jbd2", "btrfs", "zfs", "io_uring"])
}

fn is_network_daemon_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(
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
    contains_any_ignore_ascii(comm, &["pressure-vessel", "bwrap"])
}

fn is_launcher_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(
        comm,
        &["epicgames", "origin", "uplay", "battle.net", "lutris"],
    )
}

fn is_service_looking_process(comm: &str, cgroup: &str) -> bool {
    eq_ignore_ascii(comm, "systemd")
        || contains_ignore_ascii(cgroup, ".service")
        || contains_ignore_ascii(cgroup, "/system.slice/")
        || ends_with_ignore_ascii(comm, "d")
}

fn is_editor_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["code", "vscodium", "kate", "nvim", "vim", "emacs"])
}

fn is_terminal_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(
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
    eq_any_ignore_ascii(comm, &["bash", "zsh", "fish", "sh"])
}

fn is_media_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["vlc", "mpv", "spotify"])
}

fn is_recorder_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["obs", "gpu-screen-recorder", "recorder"])
}

fn is_vm_comm(comm: &str) -> bool {
    contains_any_ignore_ascii(comm, &["qemu", "virt", "virtualbox", "vmware"])
}

fn starts_with_ignore_ascii(value: &str, prefix: &str) -> bool {
    let value = value.as_bytes();
    let prefix = prefix.as_bytes();
    if prefix.len() > value.len() {
        return false;
    }

    eq_ignore_ascii_bytes(&value[..prefix.len()], prefix)
}

fn eq_any_ignore_ascii(value: &str, expected: &[&str]) -> bool {
    expected
        .iter()
        .any(|expected| eq_ignore_ascii(value, expected))
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
