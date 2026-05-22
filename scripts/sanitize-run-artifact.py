#!/usr/bin/env python3
from __future__ import annotations

import argparse
import getpass
import json
import os
import re
import shutil
import socket
import subprocess
import sys
from pathlib import Path
from typing import Any

JSON_OBJECT_FILES = (
    "session.json",
    "metadata.json",
)

NDJSON_STREAM_FILES = (
    "interval.json",
    "spike_events.json",
    "tree_events.json",
    "irq_events.json",
    "gpu_samples.json",
    "frame_correlation.json",
    "frame_events.json",
    "migration_events.json",
    "cpu_freq_samples.json",
    "io_events.json",
    "scx_events.json",
    "focus_events.json",
    "foreground_events.json",
)

OPTIONAL_TEXT_FILES = (
    "fixture.toml",
)

TITLE_KEYS = {
    "title",
    "final_title",
    "window_title",
    "wm_name",
    "net_wm_name",
}

HOST_KEYS = {
    "host",
    "hostname",
    "host_name",
    "machine",
    "machine_name",
    "node",
    "nodename",
}

USER_KEYS = {
    "user",
    "username",
    "user_name",
    "login",
    "owner",
}

PATHISH_KEY_PARTS = (
    "path",
    "cmdline",
    "command",
    "command_line",
    "exe",
    "executable",
    "argv",
    "library",
    "steam",
    "cgroup",
)

SEMANTIC_KEYS_TO_PRESERVE = {
    "task",
    "tid",
    "pid",
    "process_pid",
    "ppid",
    "process_ppid",
    "class",
    "task_class",
    "active",
    "cpu",
    "wakeup_target_cpu",
    "prio",
    "latency_ns",
    "wakeup_ns",
    "switch_ns",
    "switch_prev_pid",
    "switch_prev_state",
    "switch_prev_state_label",
    "elapsed_ms",
    "enter_ns",
    "exit_ns",
    "duration_ns",
    "timestamp_ns",
    "cpu_psi_some",
    "cpu_psi_full",
    "mem_psi_some",
    "mem_psi_full",
    "io_psi_some",
    "io_psi_full",
    "gpu_busy_percent",
    "vram_used_bytes",
    "vram_total_bytes",
    "vram_used_percent",
    "gpu_clock_mhz",
    "mem_clock_mhz",
    "temp_millidegrees",
    "power_microwatts",
    "frametime_ms",
    "irq",
    "dev",
    "sector",
    "nr_sector",
    "rwbs",
    "source",
    "status",
    "app_id",
    "workspace",
    "confidence",
}

EMAIL_RE = re.compile(r"\b[A-Za-z0-9.*%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
HOME_PATH_RE = re.compile(r"/(?:home|Users)/([^/\s:\"']+)")
RUN_USER_RE = re.compile(r"/run/user/\d+")
USER_SLICE_RE = re.compile(r"user-\d+\.slice")
USER_SERVICE_RE = re.compile(r"user@\d+\.service")
SESSION_SCOPE_RE = re.compile(r"session-\d+\.scope")
ABS_PATH_RE = re.compile(
    r"(?<![A-Za-z0-9*.-])/(?:[A-Za-z0-9.*+@=:-]+/)+[A-Za-z0-9.*+@=:-]*"
)
WINDOWS_PATH_RE = re.compile(
    r"(?i)\b[A-Z]:\\(?:[^\/:*?\"<>|\r\n]+\\)*[^\/:*?\"<>|\r\n]*"
)
STEAM_PATH_RE = re.compile(
    r"(?i)(?:[A-Za-z]:[\\/])?(?:[^\s\"']*(?:SteamLibrary|steamapps[\/]+common|.local[\/]+share[\/]+Steam)[^\s\"']*)"
)
HEX_WINDOW_RE = re.compile(r"\b0x[0-9a-fA-F]{6,}\b")

class SanitizationError(RuntimeError):
    pass

class Sanitizer:
    def __init__(self, fixture_name: str) -> None:
        self.fixture_name = fixture_name
        self.current_user = getpass.getuser()
        self.home = Path.home()
        self.home_name = self.home.name
        self.hostname = socket.gethostname()
        self.extra_usernames = self._collect_usernames()

    def _collect_usernames(self) -> set[str]:
        names = {self.current_user, self.home_name}
        try:
            login = os.getlogin()
        except OSError:
            login = ""
        if login:
            names.add(login)
        return {name for name in names if len(name) >= 3 and name != "root"}

    def sanitize_json_value(self, value: Any, key: str | None = None) -> Any:
        if isinstance(value, dict):
            sanitized: dict[str, Any] = {}
            for child_key, child_value in value.items():
                sanitized[child_key] = self.sanitize_json_value(child_value, child_key)
            if key is None:
                self._normalize_top_level_run_name(sanitized)
            return sanitized

        if isinstance(value, list):
            return [self.sanitize_json_value(item, key) for item in value]

        if isinstance(value, str):
            return self.sanitize_string(value, key)

        return value

    def _normalize_top_level_run_name(self, value: dict[str, Any]) -> None:
        if "run_name" in value:
            value["run_name"] = self.fixture_name

    def sanitize_string(self, value: str, key: str | None = None) -> str | None:
        key_lower = (key or "").lower()

        if key_lower in TITLE_KEYS:
            return self._sanitize_title(value)

        if key_lower in HOST_KEYS and value.strip():
            return "sanitized-host"

        if key_lower in USER_KEYS and value.strip():
            return "sanitized-user"

        text = value

        text = EMAIL_RE.sub("user@example.invalid", text)
        text = RUN_USER_RE.sub("/run/user/UID", text)
        text = USER_SLICE_RE.sub("user-UID.slice", text)
        text = USER_SERVICE_RE.sub("user@UID.service", text)
        text = SESSION_SCOPE_RE.sub("session-ID.scope", text)
        text = HEX_WINDOW_RE.sub("0xSANITIZED", text)

        if self.hostname and len(self.hostname) >= 3:
            text = text.replace(self.hostname, "sanitized-host")
            text = text.replace(self.hostname.lower(), "sanitized-host")
            text = text.replace(self.hostname.upper(), "SANITIZED-HOST")

        for username in self.extra_usernames:
            text = self._replace_username_token(text, username)

        text = HOME_PATH_RE.sub(self._replace_home_path_match, text)
        text = STEAM_PATH_RE.sub(self._replace_steam_path_match, text)
        text = WINDOWS_PATH_RE.sub(self._replace_windows_path_match, text)

        if self._key_looks_pathish(key_lower) or self._string_looks_like_path(text):
            text = ABS_PATH_RE.sub(self._replace_abs_path_match, text)

        return text

    def _sanitize_title(self, value: str) -> str | None:
        if not value.strip():
            return None

        generic_titles = {
            "game",
            "stutter",
            "steam",
            "gamescope",
            "benchmark",
            "unknown",
            "sanitized-window",
        }
        normalized = value.strip().lower()
        if normalized in generic_titles:
            return value.strip()

        return None

    def _key_looks_pathish(self, key_lower: str) -> bool:
        return any(part in key_lower for part in PATHISH_KEY_PARTS)

    def _string_looks_like_path(self, text: str) -> bool:
        return (
            "/" in text
            or "\\" in text
            or ".steam" in text.lower()
            or "steamapps" in text.lower()
            or "steamlibrary" in text.lower()
        )

    def _replace_username_token(self, text: str, username: str) -> str:
        token_re = re.compile(
            rf"(?<![A-Za-z0-9_.-]){re.escape(username)}(?![A-Za-z0-9_.-])"
        )
        return token_re.sub("sanitized-user", text)

    def _replace_home_path_match(self, match: re.Match[str]) -> str:
        original = match.group(0)
        if original.startswith("/Users/"):
            return "/Users/sanitized-user"
        return "/home/sanitized-user"

    def _replace_steam_path_match(self, match: re.Match[str]) -> str:
        text = match.group(0).replace("\\", "/")
        basename = self._basename_for_redacted_path(text)
        if basename:
            return f"/redacted/SteamLibrary/{basename}"
        return "/redacted/SteamLibrary"

    def _replace_windows_path_match(self, match: re.Match[str]) -> str:
        text = match.group(0)
        basename = self._basename_for_redacted_path(text.replace("\\", "/"))
        if basename:
            return f"C:\\redacted\\{basename}"
        return "C:\\redacted"

    def _replace_abs_path_match(self, match: re.Match[str]) -> str:
        text = match.group(0)

        if text.startswith("/home/sanitized-user"):
            return text
        if text.startswith("/Users/sanitized-user"):
            return text
        if text.startswith("/run/user/UID"):
            return text
        if text.startswith("/redacted/"):
            return text

        safe_prefixes = (
            "/proc/",
            "/sys/",
            "/dev/",
            "/usr/",
            "/lib/",
            "/lib64/",
            "/bin/",
            "/sbin/",
            "/etc/",
        )
        if text.startswith(safe_prefixes):
            return text

        basename = self._basename_for_redacted_path(text)
        if basename:
            return f"/redacted/path/{basename}"
        return "/redacted/path"

    def _basename_for_redacted_path(self, text: str) -> str:
        cleaned = text.strip().rstrip("/\\")
        if not cleaned:
            return ""
        basename = cleaned.replace("\\", "/").split("/")[-1]
        basename = re.sub(r"[^A-Za-z0-9._+-]", "_", basename)
        if basename in {"", ".", ".."}:
            return ""
        return basename

def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Sanitize a stutter run artifact directory into a fixture-ready directory."
        )
    )
    parser.add_argument(
        "--input",
        required=True,
        type=Path,
        help="Input run directory containing session.json and artifact streams.",
    )
    parser.add_argument(
        "--output",
        required=True,
        type=Path,
        help="Output directory to create for the sanitized fixture.",
    )
    parser.add_argument(
        "--name",
        required=True,
        help="Sanitized fixture/run name to write into session.json and metadata.json.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Remove the output directory first if it already exists.",
    )
    parser.add_argument(
        "--no-verify",
        action="store_true",
        help="Do not run cargo validate/report verification after writing output.",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=None,
        help="Repository root to use for cargo verification. Defaults to auto-detection.",
    )
    return parser.parse_args(argv)

def main(argv: list[str]) -> int:
    args = parse_args(argv)

    input_dir = args.input.resolve()
    output_dir = args.output.resolve()

    if not input_dir.is_dir():
        raise SanitizationError(f"--input is not a directory: {input_dir}")

    if output_dir.exists():
        if not args.force:
            raise SanitizationError(
                f"--output already exists: {output_dir}; pass --force to replace it"
            )
        shutil.rmtree(output_dir)

    output_dir.mkdir(parents=True, exist_ok=False)

    sanitizer = Sanitizer(args.name)

    copied_any = False
    for file_name in JSON_OBJECT_FILES:
        source = input_dir / file_name
        if source.exists():
            sanitize_json_object_file(source, output_dir / file_name, sanitizer)
            copied_any = True

    for file_name in NDJSON_STREAM_FILES:
        source = input_dir / file_name
        if source.exists():
            sanitize_ndjson_stream_file(source, output_dir / file_name, sanitizer)
            copied_any = True

    for file_name in OPTIONAL_TEXT_FILES:
        source = input_dir / file_name
        if source.exists():
            sanitize_text_file(source, output_dir / file_name, sanitizer)
            copied_any = True

    if not copied_any:
        raise SanitizationError(f"no recognized stutter artifact files found in {input_dir}")

    required_session = output_dir / "session.json"
    if not required_session.exists():
        raise SanitizationError(f"sanitized output is missing required session.json: {output_dir}")

    leaks = find_obvious_leaks(output_dir, sanitizer)
    if leaks:
        formatted = "\n".join(f"  - {leak}" for leak in leaks)
        raise SanitizationError(f"sanitized output still contains obvious private data:\n{formatted}")

    if not args.no_verify:
        repo_root = args.repo_root.resolve() if args.repo_root else find_repo_root(Path.cwd())
        run_verification(repo_root, output_dir)

    print(f"sanitized run artifact written to {output_dir}")
    return 0

def sanitize_json_object_file(source: Path, destination: Path, sanitizer: Sanitizer) -> None:
    with source.open("r", encoding="utf-8") as handle:
        value = json.load(handle)

    sanitized = sanitizer.sanitize_json_value(value)

    with destination.open("w", encoding="utf-8") as handle:
        json.dump(sanitized, handle, indent=2, sort_keys=False)
        handle.write("\n")

def sanitize_ndjson_stream_file(source: Path, destination: Path, sanitizer: Sanitizer) -> None:
    with source.open("r", encoding="utf-8") as reader, destination.open(
        "w", encoding="utf-8"
    ) as writer:
        for index, line in enumerate(reader, start=1):
            stripped = line.strip()
            if not stripped:
                continue
            try:
                value = json.loads(stripped)
            except json.JSONDecodeError as err:
                raise SanitizationError(
                    f"failed to parse NDJSON {source}:{index}: {err}"
                ) from err

            sanitized = sanitizer.sanitize_json_value(value)
            json.dump(sanitized, writer, separators=(",", ":"), sort_keys=False)
            writer.write("\n")

def sanitize_text_file(source: Path, destination: Path, sanitizer: Sanitizer) -> None:
    text = source.read_text(encoding="utf-8")
    text = sanitizer.sanitize_string(text, key=source.name)
    if text is None:
        text = ""
    destination.write_text(text, encoding="utf-8")

def find_repo_root(start: Path) -> Path:
    candidates = [start, *start.parents]
    for candidate in candidates:
        if (candidate / "Cargo.toml").is_file() and (candidate / "stutter" / "Cargo.toml").is_file():
            return candidate
    raise SanitizationError(
        "could not find repository root; pass --repo-root when running outside the repository"
    )

def run_verification(repo_root: Path, output_dir: Path) -> None:
    commands = (
        ("cargo", "run", "-p", "stutter", "--", "validate", str(output_dir)),
        (
            "cargo",
            "run",
            "-p",
            "stutter",
            "--",
            "report",
            "--analysis-json",
            str(output_dir),
        ),
    )

    for command in commands:
        subprocess.run(command, cwd=repo_root, check=True)

def find_obvious_leaks(root: Path, sanitizer: Sanitizer) -> list[str]:
    leaks: list[str] = []

    username_patterns = [
        re.compile(rf"(?<![A-Za-z0-9_.-]){re.escape(username)}(?![A-Za-z0-9_.-])")
        for username in sanitizer.extra_usernames
    ]
    hostname_pattern = (
        re.compile(re.escape(sanitizer.hostname), re.IGNORECASE)
        if sanitizer.hostname and len(sanitizer.hostname) >= 3
        else None
    )

    forbidden_patterns: list[tuple[str, re.Pattern[str]]] = [
        ("unredacted home path", re.compile(r"/home/(?!sanitized-user(?:/|$))[^/\s:\"']+")),
        ("unredacted macOS user path", re.compile(r"/Users/(?!sanitized-user(?:/|$))[^/\s:\"']+")),
        ("unredacted run user path", re.compile(r"/run/user/(?!UID(?:/|$))\d+")),
        ("email address", EMAIL_RE),
    ]

    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            leaks.append(f"{path}: non-UTF-8 file in sanitized artifact")
            continue

        for label, pattern in forbidden_patterns:
            if pattern.search(text):
                leaks.append(f"{path}: {label}")

        for pattern in username_patterns:
            if pattern.search(text):
                leaks.append(f"{path}: current username still present")

        if hostname_pattern and hostname_pattern.search(text):
            leaks.append(f"{path}: current hostname still present")

    return leaks

if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except SanitizationError as err:
        print(f"sanitize-run-artifact: error: {err}", file=sys.stderr)
        raise SystemExit(1)
