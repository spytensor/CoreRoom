#!/usr/bin/env python3
"""Run real local v0.9 CoreRoom user-case dogfood checks.

This script is intentionally heavier than unit tests. It builds the local
binary, creates a temporary user project, runs real `cr` commands against that
project, enters the full-screen console through a PTY, exercises the
non-default unified live-room composer, and regenerates README visual assets.
It is meant for release gating, not fast inner-loop testing.
"""

from __future__ import annotations

import fcntl
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "cr"
SNAPSHOT = ROOT / "tests" / "fixtures" / "console_snapshot_v08.toml"
README_IMAGES = [
    ROOT / "docs" / "images" / "boot-dashboard.png",
    ROOT / "docs" / "images" / "work-cards.png",
    ROOT / "docs" / "images" / "control-room-console.png",
]
ANSI_RE = re.compile(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\)|[=>78])")


class DogfoodFailure(RuntimeError):
    """Release-gate failure."""


def main() -> int:
    print("CoreRoom v0.9 real local user-case dogfood")
    print(f"repo: {ROOT}")

    run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, timeout=180)
    assert_file(BIN)

    version = run([str(BIN), "--version"], cwd=ROOT)
    require("cr ", version, "binary version output")

    with tempfile.TemporaryDirectory(prefix="coreroom-v09-dogfood-") as tmp:
        project = Path(tmp) / "fresh-user-project"
        create_sample_project(project)
        dogfood_fresh_project(project)
        dogfood_default_cr_entrypoint(project)
        dogfood_live_console_pty(project)
        dogfood_live_room_composer_pty(project)

    dogfood_snapshot_console_pty(width=120, height=40)
    dogfood_nerd_font_avatar_pack()
    dogfood_readme_images()

    print("\nDOGFOOD PASS: v0.9 local user-case gate completed")
    return 0


def dogfood_fresh_project(project: Path) -> None:
    print("\n== Scenario: fresh user project init")
    output = run(
        [
            str(BIN),
            "init",
            "--project",
            str(project),
            "--yes",
            "--preset",
            "team",
        ],
        cwd=ROOT,
    )
    require("wrote", output, "init wrote .coreroom")
    assert_file(project / ".coreroom" / "config.toml")
    assert_file(project / ".coreroom" / "shared.md")
    assert_file(project / ".coreroom" / "priors.lock")
    for role in ["host", "engineer", "reviewer", "sre", "security", "qa"]:
        assert_file(project / ".coreroom" / "roles" / role / "priors.md")

    roles = run([str(BIN), "role", "list", "--project", str(project)], cwd=ROOT)
    for role in ["@host", "@engineer", "@reviewer", "@sre", "@security", "@qa"]:
        require(role, roles, "team preset role list")

    prompt = run([str(BIN), "prompt", "show", "host", "--project", str(project)], cwd=ROOT)
    require("You are `@host`", prompt, "host prompt")
    require("highest-authority role inside CoreRoom", prompt, "host prompt authority")

    verify = run([str(BIN), "verify", "--project", str(project)], cwd=ROOT)
    require("verified", verify.lower(), "priors lock verification")


def dogfood_default_cr_entrypoint(project: Path) -> None:
    print("\n== Scenario: plain `cr` shows console before REPL")
    output, code = run_pty_sequence(
        [str(BIN)],
        cwd=project,
        width=120,
        height=40,
        sends=[
            (b"CoreRoom", b"q"),
            (b"permission bridge unavailable", b"\x04"),
        ],
        timeout=18,
    )
    if code != 0:
        raise DogfoodFailure(f"plain cr PTY exited with {code}\n{output}")
    for token in ["EngineeringControlRoom", "Project", "Conversation", "Roles"]:
        require(token, output, "plain cr console-first render")
    require("type a task", output, "plain cr REPL handoff")
    print("plain `cr` rendered console first, then handed off to the REPL")


def dogfood_snapshot_console_pty(width: int, height: int) -> None:
    print("\n== Scenario: real PTY snapshot console entry/exit")
    assert_file(SNAPSHOT)
    output, code = run_pty(
        [str(BIN), "console", "--snapshot", str(SNAPSHOT)],
        cwd=ROOT,
        width=width,
        height=height,
        send_when_seen=b"CoreRoom",
        send_bytes=b"q",
        timeout=12,
    )
    if code != 0:
        raise DogfoodFailure(f"console PTY exited with {code}\n{output}")
    for token in [
        "CoreRoom",
        "Project",
        "Conversation",
        "Environment",
        "Roles",
        "Evidence",
        "@host",
        "◉",
    ]:
        require(token, output, "console PTY render")
    if "@user <-> @host" not in output and "@user<->@host" not in output:
        raise DogfoodFailure("missing public user/host transcript marker in console PTY render")
    print(f"PTY console entered and exited cleanly at {width}x{height}")


def dogfood_live_console_pty(project: Path) -> None:
    print("\n== Scenario: real PTY live console without snapshot")
    live_output, live_code = run_pty(
        [str(BIN), "console"],
        cwd=project,
        width=120,
        height=40,
        send_when_seen=b"CoreRoom",
        send_bytes=b"q",
        timeout=12,
    )
    if live_code != 0:
        raise DogfoodFailure(f"live console PTY exited with {live_code}\n{live_output}")
    require("Public conversation", live_output, "live console PTY render")
    require("open evidence:0", live_output, "live console PTY render")
    print("live PTY console entered without --snapshot at 120x40")


def dogfood_live_room_composer_pty(project: Path) -> None:
    print("\n== Scenario: real PTY unified live room composer")
    output, code = run_pty_scripted_inputs(
        [str(BIN), "console", "--live-room"],
        cwd=project,
        width=160,
        height=48,
        wait_for=b"Composer",
        inputs=[
            b"validate unified room from real pty\r",
            b"@reviewer check explicit routing\r",
            b"/journal reviewer\r",
            b"/exit\r",
        ],
        timeout=24,
    )
    if code != 0:
        raise DogfoodFailure(f"live-room PTY exited with {code}\n{output}")
    for token in [
        "CoreRoom",
        "Project",
        "Conversation",
        "Composer",
    ]:
        require(token, output, "live-room PTY composer flow")
    for token in [
        "Control Rail",
        "validate unified room from real pty",
        "queued for @host",
        "@reviewer check explicit routing",
        "explicit @role mention",
        "not yet available in `cr console --live-room`",
        "cr start",
    ]:
        require_compact(token, output, "live-room PTY composer flow")
    if "CoreRoom console closed; starting REPL" in output:
        raise DogfoodFailure("live-room PTY unexpectedly fell through to the old REPL")
    print("live-room PTY accepted user input, routed @host/@reviewer, and exited in-room")


def run_pty_scripted_inputs(
    cmd: list[str],
    *,
    cwd: Path,
    width: int,
    height: int,
    wait_for: bytes,
    inputs: list[bytes],
    timeout: int,
) -> tuple[str, int]:
    print(f"$ {shell_join(cmd)}  # PTY {width}x{height}, typed composer inputs")
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
    env = os.environ.copy()
    env.update({"TERM": "xterm-256color", "COLUMNS": str(width), "LINES": str(height)})
    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        env=env,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
    )
    os.close(slave)
    output = bytearray()
    deadline = time.time() + timeout
    try:
        wait_until_seen(master, output, wait_for, deadline)
        for payload in inputs:
            os.write(master, payload)
            collect_for(master, output, seconds=0.55)
        while time.time() < deadline and proc.poll() is None:
            collect_for(master, output, seconds=0.1)
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=2)
    finally:
        os.close(master)
    text = output.decode("utf-8", errors="replace")
    cleaned = clean_terminal_text(text)
    print(indent_output(cleaned[-2400:]))
    return cleaned, proc.returncode if proc.returncode is not None else -1


def wait_until_seen(
    master: int, output: bytearray, needle: bytes, deadline: float
) -> None:
    while time.time() < deadline:
        collect_for(master, output, seconds=0.1)
        if needle in output:
            return
    raise DogfoodFailure(f"PTY did not render expected token {needle!r}")


def collect_for(master: int, output: bytearray, *, seconds: float) -> None:
    end = time.time() + seconds
    while time.time() < end:
        readable, _, _ = select.select([master], [], [], min(0.05, max(end - time.time(), 0)))
        if not readable:
            continue
        try:
            chunk = os.read(master, 4096)
        except OSError:
            return
        if not chunk:
            return
        output.extend(chunk)


def dogfood_nerd_font_avatar_pack() -> None:
    print("\n== Scenario: opt-in Nerd Font role avatar pack")
    output, code = run_pty(
        [str(BIN), "console", "--snapshot", str(SNAPSHOT)],
        cwd=ROOT,
        width=120,
        height=40,
        send_when_seen=b"CoreRoom",
        send_bytes=b"q",
        timeout=12,
        env_extra={"COREROOM_AVATAR_PACK": "nerd-font"},
    )
    if code != 0:
        raise DogfoodFailure(f"Nerd Font console PTY exited with {code}\n{output}")
    require("@host", output, "Nerd Font console PTY keeps role name")
    require("\U000f09d1", output, "Nerd Font console PTY host avatar")
    if "@user <-> @host" not in output and "@user<->@host" not in output:
        raise DogfoodFailure("Nerd Font mode polluted or removed the public conversation marker")
    print("Nerd Font avatar pack rendered through real PTY while keeping @role text")


def dogfood_readme_images() -> None:
    print("\n== Scenario: deterministic README visuals")
    run(["make", "readme-images"], cwd=ROOT, timeout=120)
    for image in README_IMAGES:
        assert_png(image)
        print(f"verified PNG: {image.relative_to(ROOT)} ({image.stat().st_size} bytes)")


def create_sample_project(project: Path) -> None:
    (project / "src").mkdir(parents=True, exist_ok=True)
    (project / "README.md").write_text("# Fresh User Project\n", encoding="utf-8")
    (project / "Cargo.toml").write_text(
        "[package]\nname = \"fresh-user-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        encoding="utf-8",
    )
    (project / "src" / "main.rs").write_text(
        "fn main() { println!(\"hello from dogfood\"); }\n",
        encoding="utf-8",
    )


def run(
    cmd: list[str],
    *,
    cwd: Path,
    timeout: int = 60,
    env: dict[str, str] | None = None,
) -> str:
    print(f"$ {shell_join(cmd)}")
    completed = subprocess.run(
        cmd,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
        check=False,
    )
    if completed.stdout:
        print(indent_output(completed.stdout))
    if completed.returncode != 0:
        raise DogfoodFailure(
            f"command failed with {completed.returncode}: {shell_join(cmd)}"
        )
    return completed.stdout


def run_pty(
    cmd: list[str],
    *,
    cwd: Path,
    width: int,
    height: int,
    send_when_seen: bytes,
    send_bytes: bytes,
    timeout: int,
    env_extra: dict[str, str] | None = None,
) -> tuple[str, int]:
    print(f"$ {shell_join(cmd)}  # PTY {width}x{height}, send {send_bytes!r}")
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
    env = os.environ.copy()
    env.update({"TERM": "xterm-256color", "COLUMNS": str(width), "LINES": str(height)})
    if env_extra:
        env.update(env_extra)
    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        env=env,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
    )
    os.close(slave)
    output = bytearray()
    sent = False
    deadline = time.time() + timeout
    try:
        while time.time() < deadline:
            readable, _, _ = select.select([master], [], [], 0.1)
            if readable:
                try:
                    chunk = os.read(master, 4096)
                except OSError:
                    break
                if not chunk:
                    break
                output.extend(chunk)
                if send_when_seen in output and not sent:
                    os.write(master, send_bytes)
                    sent = True
            if proc.poll() is not None:
                break
        if proc.poll() is None:
            if not sent:
                os.write(master, send_bytes)
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=2)
    finally:
        os.close(master)
    text = output.decode("utf-8", errors="replace")
    cleaned = clean_terminal_text(text)
    print(indent_output(cleaned[-1600:]))
    return cleaned, proc.returncode if proc.returncode is not None else -1


def run_pty_sequence(
    cmd: list[str],
    *,
    cwd: Path,
    width: int,
    height: int,
    sends: list[tuple[bytes, bytes]],
    timeout: int,
) -> tuple[str, int]:
    print(f"$ {shell_join(cmd)}  # PTY {width}x{height}, scripted sends")
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
    env = os.environ.copy()
    env.update({"TERM": "xterm-256color", "COLUMNS": str(width), "LINES": str(height)})
    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        env=env,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
    )
    os.close(slave)
    output = bytearray()
    sent = [False for _ in sends]
    deadline = time.time() + timeout
    try:
        while time.time() < deadline:
            readable, _, _ = select.select([master], [], [], 0.1)
            if readable:
                try:
                    chunk = os.read(master, 4096)
                except OSError:
                    break
                if not chunk:
                    break
                output.extend(chunk)
                for index, (trigger, payload) in enumerate(sends):
                    if not sent[index] and trigger in output:
                        os.write(master, payload)
                        sent[index] = True
            if proc.poll() is not None:
                break
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=2)
    finally:
        os.close(master)
    text = output.decode("utf-8", errors="replace")
    cleaned = clean_terminal_text(text)
    print(indent_output(cleaned[-2400:]))
    return cleaned, proc.returncode if proc.returncode is not None else -1


def assert_file(path: Path) -> None:
    if not path.exists():
        raise DogfoodFailure(f"missing expected file: {path}")


def assert_png(path: Path) -> None:
    assert_file(path)
    with path.open("rb") as handle:
        signature = handle.read(8)
    if signature != b"\x89PNG\r\n\x1a\n":
        raise DogfoodFailure(f"not a PNG: {path}")
    if path.stat().st_size < 100_000:
        raise DogfoodFailure(f"PNG unexpectedly small: {path}")


def require(needle: str, haystack: str, context: str) -> None:
    if needle not in haystack:
        raise DogfoodFailure(f"missing `{needle}` in {context}")


def require_compact(needle: str, haystack: str, context: str) -> None:
    """Require text while tolerating terminal cursor/layout whitespace loss."""
    compact_needle = re.sub(r"\s+", "", needle)
    compact_haystack = re.sub(r"\s+", "", haystack)
    if compact_needle not in compact_haystack:
        raise DogfoodFailure(f"missing `{needle}` in {context}")


def shell_join(cmd: list[str]) -> str:
    return " ".join(shlex_quote(part) for part in cmd)


def shlex_quote(value: str) -> str:
    if not value:
        return "''"
    safe = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_+-=.,/:@%"
    if all(char in safe for char in value):
        return value
    return "'" + value.replace("'", "'\"'\"'") + "'"


def indent_output(output: str) -> str:
    return "\n".join(f"  {line}" for line in output.rstrip().splitlines())


def clean_terminal_text(output: str) -> str:
    cleaned = ANSI_RE.sub("", output)
    return "".join(char if char == "\n" or ord(char) >= 32 else "" for char in cleaned)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (DogfoodFailure, subprocess.TimeoutExpired) as error:
        print(f"\nDOGFOOD FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
