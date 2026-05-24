#!/usr/bin/env python3
"""Render the PNG screenshots embedded in README.md.

The images are intentionally synthetic terminal compositions. They keep the
README stable and reproducible without requiring a live TUI, VHS, freeze, or a
particular desktop screenshot setup.

The renderer keeps the layout code at the original 1× coordinate space and
upscales every draw call by ``SCALE`` at the boundary — so the PNGs land at
retina resolution (3600 × 1800 by default) and text stays crisp on modern
displays without making every literal coordinate harder to read.
"""

from __future__ import annotations

import re
import sys
from copy import deepcopy
from pathlib import Path
from typing import Iterable

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError as exc:
    raise SystemExit(
        "Pillow is required. Install it with: python3 -m pip install --user pillow"
    ) from exc

try:
    import tomllib
except ImportError as exc:
    raise SystemExit("Python 3.11+ is required for tomllib.") from exc


ROOT = Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "docs" / "images"
CONSOLE_SNAPSHOT_FIXTURE = ROOT / "tests" / "fixtures" / "console_snapshot_v08.toml"

# Layout is authored at this base size; SCALE multiplies the actual pixel
# output so the PNGs are crisp on retina displays. Layout helpers below
# work in 1× coordinates and the `ScaledDraw` wrapper handles the
# translation when calling Pillow.
CANVAS_LOGICAL = (1800, 900)
SCALE = 2
CANVAS = (CANVAS_LOGICAL[0] * SCALE, CANVAS_LOGICAL[1] * SCALE)

BG = (0, 7, 7)
PANEL = (0, 12, 12)
CYAN = (64, 238, 224)
YELLOW = (255, 210, 38)
WHITE = (226, 226, 226)
MUTED = (145, 145, 145)
DIM = (95, 95, 95)
BLUE = (78, 166, 255)
GREEN = (54, 230, 178)
PURPLE = (164, 117, 255)
SECURITY = (45, 224, 215)
BACKEND = (55, 141, 220)
CI = (38, 190, 142)
ORANGE = (255, 169, 45)
RED = (255, 92, 80)
LIME = (168, 255, 151)
RAIL_BG = (0, 10, 10)


# Font candidate lists, checked in order. Linux paths first (CI + most
# dev boxes), macOS system paths as fallback so contributors can
# regenerate locally without installing a font package. If none match,
# Pillow drops to a default bitmap font that ignores the size argument;
# `load_font` prints a loud warning when that happens so a broken
# regeneration is loud rather than silent.
FONT_REGULAR = [
    "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf",
    "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    "/System/Library/Fonts/Menlo.ttc",
    "/System/Library/Fonts/SFNSMono.ttf",
]
FONT_BOLD = [
    "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Bold.ttf",
    "/usr/share/fonts/truetype/ubuntu/UbuntuMono-B.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
    "/System/Library/Fonts/Menlo.ttc",
    "/System/Library/Fonts/SFNSMono.ttf",
]
FONT_ITALIC = [
    "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Italic.ttf",
    "/usr/share/fonts/truetype/ubuntu/UbuntuMono-RI.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Italic.ttf",
    "/System/Library/Fonts/SFNSMonoItalic.ttf",
    "/System/Library/Fonts/Menlo.ttc",
]


def load_font(candidates: Iterable[str], size: int) -> ImageFont.ImageFont:
    """Load a truetype font at the *scaled* pixel size so retina output
    is sharp. Layout code calls this with logical sizes; the SCALE
    multiplication happens here."""
    pixel_size = size * SCALE
    for candidate in candidates:
        path = Path(candidate)
        if path.exists():
            return ImageFont.truetype(str(path), size=pixel_size)
    print(
        "WARN: no matching truetype font found — output will use Pillow's "
        "default bitmap font and ignore size hints",
        file=sys.stderr,
    )
    return ImageFont.load_default()


FONT = load_font(FONT_REGULAR, 26)
BOLD = load_font(FONT_BOLD, 26)
TITLE = load_font(FONT_BOLD, 29)
BODY = load_font(FONT_REGULAR, 24)
SMALL = load_font(FONT_REGULAR, 22)
SMALL_BOLD = load_font(FONT_BOLD, 22)
ITALIC = load_font(FONT_ITALIC, 24)


def _scale(value):
    """Recursively multiply every numeric leaf in a Pillow coordinate
    argument by SCALE. Handles scalars, tuples, and nested tuples."""
    if isinstance(value, tuple):
        return tuple(_scale(v) for v in value)
    if isinstance(value, list):
        return [_scale(v) for v in value]
    if isinstance(value, (int, float)):
        return value * SCALE
    return value


class ScaledDraw:
    """Thin shim over `ImageDraw.ImageDraw` that scales every coordinate
    argument by SCALE so the layout code can stay in logical pixels.
    ``textbbox`` returns logical-space coordinates (the Pillow result
    is divided back) so text-width math composes cleanly with the rest
    of the layout."""

    def __init__(self, draw: ImageDraw.ImageDraw) -> None:
        self._draw = draw

    def text(self, xy, *args, **kwargs):
        return self._draw.text(_scale(xy), *args, **kwargs)

    def line(self, xy, *args, **kwargs):
        return self._draw.line(_scale(xy), *args, **kwargs)

    def rectangle(self, xy, *args, **kwargs):
        return self._draw.rectangle(_scale(xy), *args, **kwargs)

    def ellipse(self, xy, *args, **kwargs):
        return self._draw.ellipse(_scale(xy), *args, **kwargs)

    def textbbox(self, xy, *args, **kwargs):
        bbox = self._draw.textbbox(_scale(xy), *args, **kwargs)
        return tuple(c // SCALE for c in bbox)


def text_width(draw: ScaledDraw, text: str, font: ImageFont.ImageFont) -> int:
    left, _, right, _ = draw.textbbox((0, 0), text, font=font)
    return right - left


def fit_text(
    draw: ScaledDraw, text: str, max_width: int, font: ImageFont.ImageFont
) -> str:
    if text_width(draw, text, font) <= max_width:
        return text
    suffix = "..."
    available = max_width - text_width(draw, suffix, font)
    clipped = ""
    for char in text:
        if text_width(draw, clipped + char, font) > available:
            break
        clipped += char
    return clipped.rstrip() + suffix


def draw_text(
    draw: ScaledDraw,
    xy: tuple[int, int],
    text: str,
    color: tuple[int, int, int] = WHITE,
    font: ImageFont.ImageFont = FONT,
) -> None:
    draw.text(xy, text, fill=color, font=font)


def package_version() -> str:
    manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'(?m)^version\s*=\s*"([^"]+)"', manifest)
    if not match:
        raise SystemExit("Could not find package version in Cargo.toml")
    return match.group(1)


def splash_copy(version: str) -> tuple[list[str], str, list[str]]:
    with (ROOT / "data" / "splash_content.toml").open("rb") as handle:
        data = tomllib.load(handle)
    tips = list(data.get("tips", {}).get("items", []))
    entries = list(data.get("whats_new", []))
    if not entries:
        return tips, version, []
    chosen = next((entry for entry in entries if entry.get("version") == version), entries[0])
    return tips, str(chosen.get("version", version)), list(chosen.get("items", []))


def console_snapshot_fixture() -> dict:
    with CONSOLE_SNAPSHOT_FIXTURE.open("rb") as handle:
        return tomllib.load(handle)


def readme_console_snapshot() -> dict:
    """Project the v0.8 fixture into the current v0.9 README story.

    The fixture remains the structural test packet for v0.8 dogfood. README
    images should show the current full-screen console surface, so this helper
    updates the synthetic visible facts without mutating the fixture file.
    """
    snapshot = deepcopy(console_snapshot_fixture())
    project = snapshot["project"]
    project["branch"] = "feat/v0.9-261-terminal-qa-readme-visuals"
    project["dirtyState"] = "dirty"
    project["version"] = "0.9.0-dev"
    project["activePhase"] = "v0.9.0 - Full-screen Console"
    project["trackerIssue"] = 239

    runtime = snapshot["runtime"]
    runtime["activeRole"] = "host"
    runtime["waitingApproval"] = True
    for role in runtime.get("roles", []):
        role_name = role.get("role")
        if role_name == "host":
            role["state"] = "working"
            role["currentWorkOrder"] = "WO-0261"
            role["currentGatePhase"] = "qa"
            role["lastActivity"] = "Collecting terminal QA evidence"
        elif role_name == "qa":
            role["state"] = "reviewing"
            role["currentWorkOrder"] = "WO-0261"
            role["currentGatePhase"] = "qa"
            role["lastActivity"] = "Checking render widths and overlays"
        elif role_name == "reviewer":
            role["state"] = "idle"
            role["currentWorkOrder"] = "WO-0261"
            role["currentGatePhase"] = "review"
            role["lastActivity"] = "Available for README visual review"

    snapshot["work"] = [
        {
            "id": "WO-0261",
            "title": "Terminal QA and README visuals",
            "phase": "v0.9",
            "epic": "fullscreen-console",
            "githubIssue": 261,
            "branch": "feat/v0.9-261-terminal-qa-readme-visuals",
            "ciState": "unknown",
            "evidenceState": "warn",
            "trackerState": "warn",
            "lifecycle": "in-progress",
            "sourceCitations": ["tracker:#239", "issue:#261", "amendment:A-020"],
        },
        {
            "id": "WO-0260",
            "title": "Host action overlay",
            "phase": "v0.9",
            "epic": "fullscreen-console",
            "githubIssue": 260,
            "branch": "feat/v0.9-260-console-actions",
            "pullRequest": 289,
            "ciState": "ok",
            "evidenceState": "ok",
            "trackerState": "ok",
            "lifecycle": "closed",
            "sourceCitations": ["pr:#289", "tracker:#239"],
        },
        {
            "id": "WO-0259",
            "title": "Navigation and detail panes",
            "phase": "v0.9",
            "epic": "fullscreen-console",
            "githubIssue": 259,
            "branch": "feat/v0.9-259-console-navigation",
            "pullRequest": 288,
            "ciState": "ok",
            "evidenceState": "ok",
            "trackerState": "ok",
            "lifecycle": "closed",
            "sourceCitations": ["pr:#288", "tracker:#239"],
        },
        {
            "id": "WO-0258",
            "title": "Xray and CREP logs",
            "phase": "v0.9",
            "epic": "fullscreen-console",
            "githubIssue": 258,
            "branch": "feat/v0.9-258-console-xray",
            "pullRequest": 287,
            "ciState": "ok",
            "evidenceState": "ok",
            "trackerState": "ok",
            "lifecycle": "closed",
            "sourceCitations": ["pr:#287", "tracker:#239"],
        },
    ]
    snapshot["gates"] = [
        {
            "workOrder": "WO-0261",
            "currentPhase": "qa",
            "missingReviews": [],
            "signoffReady": True,
        },
        {
            "workOrder": "WO-0260",
            "currentPhase": "closed",
            "missingReviews": [],
            "signoffReady": True,
        },
    ]
    snapshot["evidence"] = [
        {
            "workOrder": "WO-0261",
            "status": "incomplete",
            "missingFields": ["PR", "CI", "tracker update"],
            "unverifiedItems": ["Manual visual QA evidence is still being collected."],
            "rollback": "Revert the terminal QA and README visual refresh PR.",
            "trackerUpdated": False,
        },
        {
            "workOrder": "WO-0260",
            "status": "complete",
            "rollback": "Revert PR #289; action overlay only.",
            "trackerUpdated": True,
        },
    ]
    snapshot["sources"] = [
        {
            "sourceId": "core-repo",
            "status": "pinned",
            "pin": "commit:v0.9-console-head",
            "trustLevel": "project-source",
            "visibleRoles": ["host", "engineer", "reviewer", "qa"],
            "findings": ["Console renderer, navigation, action overlay, and tests are local facts."],
            "relatedWorkOrders": ["WO-0261"],
        },
        {
            "sourceId": "readme-images",
            "status": "pinned",
            "pin": "script:scripts/render-readme-images.py",
            "trustLevel": "generated",
            "visibleRoles": ["host", "qa"],
            "findings": ["README images are regenerated from this deterministic renderer."],
            "relatedWorkOrders": ["WO-0261"],
        },
    ]
    snapshot["alerts"] = [
        {
            "id": "qa:visual",
            "title": "Terminal QA evidence required",
            "severity": "warn",
            "source": "issue:#261",
        },
        {
            "id": "evidence:tracker",
            "title": "Tracker checkbox pending",
            "severity": "blocking",
            "source": "tracker:#239",
        },
    ]
    return snapshot


def first_active_work(snapshot: dict) -> dict:
    for work in snapshot.get("work", []):
        if work.get("lifecycle") != "closed":
            return work
    return snapshot.get("work", [{}])[0]


def work_color(lifecycle: str) -> tuple[int, int, int]:
    return {
        "closed": DIM,
        "blocked": ORANGE,
        "failed-ci": RED,
        "merged-tracker-stale": ORANGE,
        "in-review": CYAN,
        "in-progress": GREEN,
        "ready": BLUE,
    }.get(lifecycle, WHITE)


def role_color(role: str, state: str) -> tuple[int, int, int]:
    if role == "host":
        return PURPLE
    if state in {"blocked", "waiting-approval", "stale-session"}:
        return ORANGE
    if state == "reviewing":
        return CYAN
    if state == "working":
        return GREEN
    return BLUE


def source_color(status: str) -> tuple[int, int, int]:
    return {
        "pinned": GREEN,
        "stale": ORANGE,
        "missing": RED,
        "trust-changed": RED,
        "visibility-denied": RED,
    }.get(status, WHITE)


def evidence_color(status: str, tracker_updated: bool) -> tuple[int, int, int]:
    if not tracker_updated:
        return ORANGE
    return {
        "complete": GREEN,
        "incomplete": ORANGE,
        "missing": RED,
        "unverified": ORANGE,
    }.get(status, WHITE)


def count_where(items: list[dict], key: str, value: str) -> int:
    return sum(1 for item in items if item.get(key) == value)


def ascii_or(text: str, fallback: str) -> str:
    try:
        text.encode("ascii")
    except UnicodeEncodeError:
        return fallback
    return text


def new_canvas() -> tuple[Image.Image, ScaledDraw]:
    image = Image.new("RGB", CANVAS, BG)
    draw = ScaledDraw(ImageDraw.Draw(image))
    return image, draw


def bullet(draw: ScaledDraw, x: int, y: int, color: tuple[int, int, int]) -> None:
    draw.ellipse((x, y, x + 14, y + 14), fill=color)


def prompt(draw: ScaledDraw, x: int, y: int, body: str) -> None:
    draw_text(draw, (x, y), "⚡", YELLOW, BOLD)
    draw_text(draw, (x + 36, y), "cr", CYAN, BOLD)
    draw_text(draw, (x + 89, y), "›", GREEN, BOLD)
    draw_text(draw, (x + 118, y), body, WHITE, FONT)


def render_boot_dashboard() -> None:
    version = package_version()
    tips, whats_new_version, whats_new = splash_copy(version)
    image, draw = new_canvas()

    left, top, right, bottom = 90, 92, 1440, 653
    draw.rectangle((left, top, right, bottom), fill=PANEL, outline=CYAN, width=2)
    title = f" CoreRoom v{version} "
    draw.rectangle((left + 30, top - 21, left + 30 + text_width(draw, title, TITLE), top + 9), fill=BG)
    draw_text(draw, (left + 36, top - 19), title, CYAN, TITLE)

    draw_text(draw, (136, 178), "welcome back, Ada", WHITE, FONT)
    roles = [
        ("@host", "cc", "1M", PURPLE),
        ("@backend", "cc", "1M", BLUE),
        ("@security", "codex", "default", SECURITY),
        ("@ci", "cc", "default", CI),
    ]
    y = 243
    for role, engine, model, color in roles:
        bullet(draw, 137, y + 6, color)
        draw_text(draw, (163, y), role, color, FONT)
        draw_text(draw, (338, y), engine, MUTED, FONT)
        draw_text(draw, (447, y), "·", DIM, FONT)
        draw_text(draw, (479, y), model, MUTED, FONT)
        y += 43

    draw.rectangle((136, 427, 209, 464), fill=CYAN)
    draw_text(draw, (148, 430), "1.8k", BG, BOLD)
    draw_text(draw, (237, 431), "base tokens loaded", WHITE, FONT)
    draw_text(draw, (136, 489), "~/codes/CoreRoom", MUTED, FONT)

    right_x = 630
    draw_text(draw, (right_x, 176), "tips for getting started", YELLOW, TITLE)
    y = 216
    for item in tips[:3]:
        draw_text(draw, (right_x, y), "•", WHITE, BODY)
        draw_text(draw, (right_x + 30, y), fit_text(draw, item, 780, BODY), WHITE, BODY)
        y += 39

    y += 18
    draw_text(draw, (right_x, y), f"what's new in {whats_new_version}", YELLOW, TITLE)
    y += 41
    for item in whats_new[:3]:
        draw_text(draw, (right_x, y), "•", WHITE, BODY)
        draw_text(draw, (right_x + 30, y), fit_text(draw, item, 780, BODY), WHITE, BODY)
        y += 39

    draw_text(draw, (right_x, 539), "/help for commands", MUTED, FONT)
    draw_text(draw, (132, 714), "type a task · @role · /help · /exit", MUTED, SMALL)
    prompt(draw, 88, 754, "@host validate the v0.9 console before release")

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    image.save(OUT_DIR / "boot-dashboard.png")


def status_line(draw: ScaledDraw, y: int, role: str) -> None:
    _ = role
    line = "  3 roles working · ... @host · recording gate evidence    ... @security · waiting approval"
    draw_text(draw, (96, y), fit_text(draw, line, 1040, FONT), MUTED, FONT)


def active_card(
    draw: ScaledDraw,
    y: int,
    role: str,
    title: str,
    state: str,
    rows: list[tuple[str, tuple[int, int, int], str]],
    color: tuple[int, int, int],
) -> None:
    left, right = 126, 1138
    height = 86 + 37 * len(rows)
    draw.line((left, y, right, y), fill=color, width=2)
    draw.line((left, y, left, y + height), fill=color, width=2)
    draw.line((right, y, right, y + height), fill=color, width=2)
    draw.line((left, y + height, right, y + height), fill=color, width=2)

    label = f" {role} working · {title} "
    draw.rectangle(
        (left + 24, y - 15, left + 24 + text_width(draw, label, FONT), y + 16),
        fill=BG,
    )
    draw_text(draw, (left + 27, y - 17), label, color, FONT)
    draw_text(draw, (left + 42, y + 34), state, WHITE, FONT)

    row_y = y + 75
    for glyph, glyph_color, text in rows:
        draw_text(draw, (left + 42, row_y), glyph, glyph_color, BOLD)
        draw_text(draw, (left + 77, row_y), fit_text(draw, text, right - left - 120, FONT), MUTED, FONT)
        row_y += 37


def permission_card(
    draw: ScaledDraw,
    y: int,
    role: str,
    title: str,
    color: tuple[int, int, int],
) -> None:
    active_card(
        draw,
        y,
        role,
        title,
        "waiting for your approval",
        [
            ("✓", GREEN, "read src/console_tui.rs and scripts/render-readme-images.py"),
            ("?", YELLOW, "wants Bash `cargo test --test console_terminal_qa_test` — [a]llow · [s]ession · [d]eny"),
        ],
        color,
    )


def done_summary(
    draw: ScaledDraw,
    y: int,
    role: str,
    title: str,
    elapsed: str,
    steps: int,
    color: tuple[int, int, int],
) -> None:
    _ = (title, color)
    draw_text(draw, (126, y), fit_text(draw, f"{role} done · {elapsed} · {steps} steps", 1040, FONT), DIM, FONT)


def chat_line(
    draw: ScaledDraw,
    y: int,
    role: str,
    text: str,
    color: tuple[int, int, int],
) -> None:
    draw_text(draw, (126, y), role, color, BOLD)
    draw_text(draw, (164, y + 38), fit_text(draw, text, 980, FONT), WHITE, FONT)


def reply_quote(
    draw: ScaledDraw,
    y: int,
    child_role: str,
    parent_role: str,
    snippet: str,
    child_color: tuple[int, int, int],
    parent_color: tuple[int, int, int],
) -> None:
    """One-line reply pointer — mirrors `format_reply_quote`."""
    draw_text(draw, (126, y), child_role, child_color, BOLD)
    arrow_x = 126 + text_width(draw, child_role, BOLD) + 14
    draw_text(draw, (arrow_x, y), "↲", DIM, FONT)
    parent_x = arrow_x + text_width(draw, "↲", FONT) + 14
    draw_text(draw, (parent_x, y), parent_role, parent_color, FONT)
    sep_x = parent_x + text_width(draw, parent_role, FONT) + 14
    draw_text(draw, (sep_x, y), "·", DIM, FONT)
    snippet_text = f'"{snippet}"'
    draw_text(draw, (sep_x + 28, y), fit_text(draw, snippet_text, 870, FONT), DIM, FONT)


def handoff_banner(
    draw: ScaledDraw,
    y: int,
    role: str,
    color: tuple[int, int, int],
) -> None:
    """Full-width handoff divider — mirrors `handoff_banner` in
    src/repl/render.rs. Painted when a TurnDispatched fires with
    queue_position == 0 (the new speaker actually starts)."""
    draw_text(draw, (126, y), role, color, BOLD)
    dash_start = 126 + text_width(draw, role, BOLD) + 14
    dash_end = 1144
    mid_y = y + 15
    draw.line(
        (dash_start, mid_y, dash_end - text_width(draw, " starting", FONT) - 12, mid_y),
        fill=DIM,
        width=1,
    )
    status_x = dash_end - text_width(draw, "starting", FONT)
    draw_text(draw, (status_x, y), "starting", MUTED, FONT)


def right_rail(draw: ScaledDraw) -> None:
    x = 1244
    draw.rectangle((1204, 128, 1726, 764), fill=RAIL_BG)
    draw_text(draw, (x, 149), "v0.9 console rail", YELLOW, TITLE)
    draw_text(draw, (x, 197), "Progress", WHITE, FONT)
    draw_text(draw, (x + 26, 230), "WorkOrder, PR, CI, tracker", MUTED, SMALL)
    draw_text(draw, (x + 26, 264), "from structural evidence", MUTED, SMALL)
    draw_text(draw, (x, 319), "Environment", WHITE, FONT)
    draw_text(draw, (x + 26, 352), "repo, branch, phase, host", MUTED, SMALL)
    draw_text(draw, (x + 26, 386), "always visible, never chat-only", MUTED, SMALL)
    draw_text(draw, (x, 441), "Subagents", WHITE, FONT)
    draw_text(draw, (x + 26, 474), "internal unless user @mentions", MUTED, SMALL)
    draw_text(draw, (x + 26, 508), "Xray holds delegation details", MUTED, SMALL)
    draw_text(draw, (x, 563), "Host action", WHITE, FONT)
    draw_text(draw, (x + 26, 596), "confirmation overlay first", MUTED, SMALL)
    draw_text(draw, (x + 26, 630), "no direct console mutation", MUTED, SMALL)
    draw_text(draw, (x, 690), "Visual QA", YELLOW, TITLE)
    draw_text(draw, (x + 26, 735), "80 / 120 / 160 / 220 cols", MUTED, FONT)


def console_box(
    draw: ScaledDraw,
    xy: tuple[int, int, int, int],
    title: str,
    color: tuple[int, int, int] = CYAN,
) -> None:
    left, top, right, bottom = xy
    draw.rectangle((left, top, right, bottom), fill=(0, 5, 5), outline=color, width=2)
    label = f" {title} "
    draw.rectangle(
        (left + 22, top - 17, left + 22 + text_width(draw, label, SMALL_BOLD), top + 12),
        fill=BG,
    )
    draw_text(draw, (left + 27, top - 18), label, color, SMALL_BOLD)


def console_header_metric(
    draw: ScaledDraw,
    x: int,
    y: int,
    label: str,
    value: str,
    value_color: tuple[int, int, int] = WHITE,
) -> None:
    draw_text(draw, (x, y), f"{label}:", ORANGE, FONT)
    draw_text(draw, (x + 140, y), value, value_color, BOLD)


def console_nav(draw: ScaledDraw) -> None:
    nav = [
        (624, 24, "<0>", "overview", CYAN, WHITE),
        (792, 24, "<1>", "roles", PURPLE, MUTED),
        (936, 24, "<2>", "workorders", BLUE, MUTED),
        (1168, 24, "<3>", "gates", GREEN, MUTED),
        (1316, 24, "<4>", "prs", YELLOW, MUTED),
        (624, 66, "<5>", "ci", CI, MUTED),
        (792, 66, "<6>", "sources", SECURITY, MUTED),
        (980, 66, "<7>", "evidence", ORANGE, MUTED),
        (1190, 66, "<8>", "logs", MUTED, MUTED),
    ]
    for x, y, hotkey, label, color, label_color in nav:
        draw_text(draw, (x, y), hotkey, color, SMALL_BOLD)
        draw_text(draw, (x + 58, y), label, label_color, SMALL)

    draw_text(draw, (624, 112), "<tab>", BLUE, SMALL_BOLD)
    draw_text(draw, (707, 112), "next", MUTED, SMALL)
    draw_text(draw, (812, 112), "<enter>", BLUE, SMALL_BOLD)
    draw_text(draw, (918, 112), "open", MUTED, SMALL)


def draw_pulse_bars(
    draw: ScaledDraw,
    x: int,
    y: int,
    width: int,
    values: list[tuple[tuple[int, int, int], int]],
    block: int = 13,
    gap: int = 10,
) -> None:
    max_blocks = max(1, width // (block + gap))
    drawn = 0
    for color, count in values:
        for _ in range(count):
            if drawn >= max_blocks:
                return
            left = x + drawn * (block + gap)
            draw.rectangle((left, y, left + block, y + 64), fill=color)
            drawn += 1


def mini_tile(
    draw: ScaledDraw,
    x: int,
    y: int,
    name: str,
    count: str,
    bars: list[tuple[tuple[int, int, int], int]],
    selected: bool = False,
) -> None:
    if selected:
        draw.rectangle((x - 18, y + 88, x + 230, y + 121), fill=CYAN)
        draw_text(draw, (x, y + 91), f"{name} - {count}", BG, SMALL_BOLD)
    else:
        draw_text(draw, (x, y + 91), f"{name} - ", WHITE, SMALL)
        draw_text(draw, (x + text_width(draw, f"{name} - ", SMALL), y + 91), count, GREEN, SMALL_BOLD)
    draw_pulse_bars(draw, x + 35, y, 218, bars, block=11, gap=8)


def status_table(draw: ScaledDraw) -> None:
    x, y = 90, 446
    draw_text(draw, (x, y), "active workorders", YELLOW, TITLE)
    rows = [
        ("#248", "v0.9 console mockup", "plan", "@host", GREEN),
        ("#246", "source drift policy", "qa", "@reviewer", CYAN),
        ("#241", "evidence packet hardening", "review", "@security", ORANGE),
        ("#237", "README visual refresh", "closed", "@host", DIM),
    ]
    y += 48
    for issue, title, gate, owner, color in rows:
        draw_text(draw, (x, y), issue, BLUE, BOLD)
        draw_text(draw, (x + 88, y), fit_text(draw, title, 390, FONT), WHITE, FONT)
        draw_text(draw, (x + 515, y), gate, color, FONT)
        draw_text(draw, (x + 645, y), owner, PURPLE if owner == "@host" else MUTED, FONT)
        y += 42


def gate_pipeline(draw: ScaledDraw) -> None:
    x, y = 90, 638
    draw_text(draw, (x, y), "gate pipeline", YELLOW, TITLE)
    phase_order = ["intake", "discovery", "plan", "review", "signoff", "implement", "qa", "closed"]
    current_phase = first_gate.get("currentPhase", "qa")
    current_index = phase_order.index(current_phase) if current_phase in phase_order else 0
    stages = []
    for index, label in enumerate(phase_order):
        if label == "closed" and current_phase != "closed":
            color = DIM
        elif index < current_index:
            color = GREEN
        elif index == current_index:
            color = ORANGE
        else:
            color = DIM
        stages.append((label, color))
    cursor = x
    y += 52
    stage_widths = {
        "intake": 100,
        "discovery": 134,
        "plan": 92,
        "review": 106,
        "signoff": 112,
        "implement": 126,
        "qa": 62,
        "closed": 90,
    }
    for label, color in stages:
        w = stage_widths[label]
        draw.rectangle((cursor, y, cursor + w, y + 43), outline=color, width=2)
        draw_text(draw, (cursor + 11, y + 7), label, color, SMALL_BOLD)
        cursor += w + 17
        if label != "closed":
            draw_text(draw, (cursor - 17, y + 6), "→", MUTED, SMALL_BOLD)


def role_lanes(draw: ScaledDraw) -> None:
    x, y = 1038, 446
    draw_text(draw, (x, y), "role lanes", YELLOW, TITLE)
    rows = [
        ("@host", "driving v0.9 console plan", PURPLE, "live"),
        ("@engineer", "waiting for context pack", BLUE, "ready"),
        ("@security", "reviewing source trust model", SECURITY, "review"),
        ("@qa", "blocked: tracker evidence missing", ORANGE, "blocked"),
    ]
    y += 51
    for role, task, color, state in rows:
        bullet(draw, x, y + 8, color)
        draw_text(draw, (x + 28, y), role, color, BOLD)
        draw_text(draw, (x + 190, y), fit_text(draw, task, 398, SMALL), WHITE, SMALL)
        draw_text(draw, (x + 616, y), state, color, SMALL_BOLD)
        y += 43


def evidence_stack(draw: ScaledDraw) -> None:
    x, y = 1088, 650
    draw_text(draw, (x, y), "evidence stack", YELLOW, TITLE)
    items = [
        ("GitHub issue", "#248 linked", GREEN),
        ("PR / CI", "pending", ORANGE),
        ("tracker", "checkbox + ledger required", ORANGE),
        ("rollback", "revert PR, no migration", GREEN),
    ]
    y += 43
    for label, value, color in items:
        draw_text(draw, (x, y), label, MUTED, SMALL)
        draw_text(draw, (x + 205, y), fit_text(draw, value, 415, SMALL_BOLD), color, SMALL_BOLD)
        y += 30


def render_control_room_console() -> None:
    snapshot = readme_console_snapshot()
    project = snapshot["project"]
    runtime = snapshot["runtime"]
    work_items = snapshot.get("work", [])
    gates = snapshot.get("gates", [])
    evidence_items = snapshot.get("evidence", [])
    sources = snapshot.get("sources", [])
    alerts = snapshot.get("alerts", [])
    conversation = snapshot.get("conversation", {})
    public_turns = conversation.get("publicTurns", [])
    internal_activity = conversation.get("internalActivity", [])
    active_work = first_active_work(snapshot)

    logical = (2400, 1350)
    image = Image.new("RGB", _scale(logical), BG)
    draw = ScaledDraw(ImageDraw.Draw(image))

    def box(xy: tuple[int, int, int, int], title: str, color: tuple[int, int, int] = CYAN) -> None:
        console_box(draw, xy, title, color)

    def metric(x: int, y: int, label: str, value: str, color: tuple[int, int, int] = WHITE) -> None:
        draw_text(draw, (x, y), f"{label}:", ORANGE, FONT)
        draw_text(draw, (x + 142, y), value, color, BOLD)

    def nav_item(x: int, y: int, key: str, label: str, color: tuple[int, int, int], active: bool = False) -> None:
        draw_text(draw, (x, y), key, color, BOLD)
        draw_text(draw, (x + 62, y), label, WHITE if active else MUTED, FONT)

    # Header: fixture-backed project facts and navigation, not decorative status.
    metric(42, 28, "Project", project["project"], CYAN)
    metric(42, 72, "Repo", project["repository"], WHITE)
    metric(42, 116, "Branch", project["branch"], GREEN)
    metric(42, 160, "Phase", project["activePhase"], YELLOW)
    metric(42, 204, "Host", f"@{runtime['hostRole']} · highest room authority", PURPLE)

    nav_item(724, 34, "<0>", "room", CYAN, True)
    nav_item(900, 34, "<1>", "roles", PURPLE)
    nav_item(1080, 34, "<2>", "workorders", BLUE)
    nav_item(1340, 34, "<3>", "gates", GREEN)
    nav_item(1518, 34, "<4>", "evidence", ORANGE)
    nav_item(724, 84, "<5>", "sources", SECURITY)
    nav_item(920, 84, "<6>", "prs/ci", CI)
    nav_item(1110, 84, "<7>", "logs", MUTED)
    nav_item(1264, 84, "<8>", "release", YELLOW)
    draw_text(draw, (724, 154), "<tab>", BLUE, BOLD)
    draw_text(draw, (812, 154), "next view", MUTED, FONT)
    draw_text(draw, (970, 154), "<enter>", BLUE, BOLD)
    draw_text(draw, (1090, 154), "open selected", MUTED, FONT)
    draw_text(draw, (1325, 154), "<space>", BLUE, BOLD)
    draw_text(draw, (1444, 154), "ask @host", MUTED, FONT)

    draw_text(draw, (1928, 38), "CoreRoom", ORANGE, TITLE)
    draw_text(draw, (1928, 82), "Engineering Control Room", WHITE, BOLD)
    draw_text(draw, (1928, 123), "for AI Agents", CYAN, BOLD)
    draw_text(
        draw,
        (1928, 190),
        fit_text(draw, f"v{project['version']} · terminal QA snapshot", 385, SMALL_BOLD),
        GREEN,
        SMALL_BOLD,
    )

    box(
        (24, 274, 2376, 1266),
        "CoreRoom Console · v0.9 full-screen control surface",
        CYAN,
    )

    # Top row: the minimum actionable pulse data for the room.
    pulse_y = 326
    enabled_roles = sum(1 for role in runtime.get("roles", []) if role.get("enabled", True))
    active_roles = sum(
        1
        for role in runtime.get("roles", [])
        if role.get("state") not in {"enabled", "idle"}
    )
    blocked_work = count_where(work_items, "lifecycle", "blocked")
    closed_work = count_where(work_items, "lifecycle", "closed")
    stale_sources = sum(1 for source in sources if source.get("status") != "pinned")
    stale_evidence = sum(
        1
        for item in evidence_items
        if item.get("status") != "complete" or not item.get("trackerUpdated", False)
    )
    pulses = [
        ("Roles", f"{enabled_roles} enabled · {active_roles} active", [(LIME, enabled_roles), (ORANGE, active_roles)], CYAN),
        ("WorkOrders", f"{len(work_items)} total · {blocked_work} blocked", [(LIME, closed_work), (ORANGE, blocked_work), (DIM, max(1, len(work_items) - closed_work - blocked_work))], BLUE),
        ("Gate", gates[0].get("currentPhase", "unknown") if gates else "unknown", [(GREEN, 4), (ORANGE, len(gates)), (DIM, 3)], GREEN),
        ("Evidence", f"{stale_evidence} closure gaps", [(LIME, max(1, len(evidence_items) - stale_evidence)), (ORANGE, stale_evidence), (DIM, 2)], ORANGE),
        ("Sources", f"{len(sources)} sources · {stale_sources} stale", [(LIME, max(1, len(sources) - stale_sources)), (RED, stale_sources), (DIM, 2)], SECURITY),
    ]
    x = 74
    for label, value, bars, color in pulses:
        draw_pulse_bars(draw, x + 50, pulse_y, 260, bars, block=14, gap=9)
        draw_text(draw, (x, pulse_y + 96), label, WHITE, FONT)
        draw_text(draw, (x, pulse_y + 135), value, color, SMALL_BOLD)
        x += 455

    # Left: project facts and role inventory. These are stable anchors.
    box((58, 520, 548, 1126), "Project State", BLUE)
    first_gate = gates[0] if gates else {}
    state_rows = [
        ("repo", project["repository"]),
        ("work", f"#{active_work.get('githubIssue', '—')} {active_work.get('title', 'no active work')}"),
        ("tracker", f"#{project['trackerIssue']}"),
        ("gate", first_gate.get("currentPhase", "none")),
        ("branch", project["branch"]),
        ("dirty", project["dirtyState"]),
    ]
    y = 570
    for label, value in state_rows:
        draw_text(draw, (90, y), label, MUTED, SMALL)
        draw_text(draw, (220, y), fit_text(draw, value, 300, SMALL_BOLD), WHITE, SMALL_BOLD)
        y += 43

    draw_text(draw, (90, 856), "enabled roles", YELLOW, TITLE)
    roles = runtime.get("roles", [])[:6]
    y = 910
    for role in roles:
        role_name = role["role"]
        engine = role["engine"]
        state = role["state"]
        color = role_color(role_name, state)
        bullet(draw, 92, y + 9, color)
        draw_text(draw, (120, y), f"@{role_name}", color, SMALL_BOLD)
        draw_text(draw, (276, y), engine, MUTED, SMALL)
        draw_text(draw, (352, y), fit_text(draw, state, 160, SMALL), WHITE, SMALL)
        y += 38

    # Center: the real CLI room. Conversation remains the primary surface.
    box((586, 520, 1668, 1126), "Conversation · @host orchestration", PURPLE)
    user_turn = public_turns[0] if public_turns else {"body": "@host inspect the room"}
    host_turn = public_turns[1] if len(public_turns) > 1 else {"body": "I will preserve a clean public transcript."}
    user_body = ascii_or(
        user_turn.get("body", ""),
        "@host validate the v0.9 console terminal QA and README visuals",
    )
    host_body = ascii_or(
        host_turn.get("body", ""),
        "I will keep the public session clear and project facts in side rails.",
    )
    prompt(draw, 622, 570, fit_text(draw, user_body, 900, FONT))
    y = 614
    draw_text(draw, (622, y), "@host", PURPLE, BOLD)
    draw_text(
        draw,
        (622, y + 42),
        fit_text(
            draw,
            "classification: persistent WorkOrder · UI/control-surface design · no release action",
            980,
            FONT,
        ),
        WHITE,
        FONT,
    )
    draw_text(
        draw,
        (622, y + 84),
        fit_text(draw, host_body, 980, FONT),
        WHITE,
        FONT,
    )
    y += 146
    card_left, card_right, card_top, card_bottom = 622, 1614, y, y + 174
    draw.rectangle((card_left, card_top, card_right, card_bottom), outline=PURPLE, width=2)
    label = " @host working · delegate design review "
    draw.rectangle(
        (card_left + 24, card_top - 16, card_left + 24 + text_width(draw, label, FONT), card_top + 15),
        fill=BG,
    )
    draw_text(draw, (card_left + 28, card_top - 18), label, PURPLE, FONT)
    draw_text(draw, (card_left + 34, card_top + 30), "building a minimal, actionable room layout", WHITE, FONT)
    card_rows = [
        ("✓", GREEN, "project, branch, phase, tracker, and host remain visible"),
        ("✓", GREEN, "conversation stays the largest center panel"),
        ("…", WHITE, f"{conversation.get('internalDelegationCount', 0)} internal delegations stay out of public chat"),
    ]
    row_y = card_top + 74
    for glyph, color, text in card_rows:
        draw_text(draw, (card_left + 34, row_y), glyph, color, BOLD)
        draw_text(draw, (card_left + 72, row_y), fit_text(draw, text, 850, FONT), MUTED, FONT)
        row_y += 35

    y = card_bottom + 30
    draw_text(draw, (622, y), "host-managed side rail", YELLOW, BOLD)
    draw_text(
        draw,
        (622, y + 38),
        fit_text(
            draw,
            "Specialist roles do not enter the public transcript unless the user @mentions them or @host surfaces a veto/risk/final evidence.",
            980,
            FONT,
        ),
        WHITE,
        FONT,
    )
    draw_text(
        draw,
        (622, y + 86),
        fit_text(
            draw,
            f"{len(internal_activity)} role activities are captured in side rails/Xray, not appended as chat.",
            980,
            SMALL,
        ),
        DIM,
        SMALL,
    )

    # Right: active work and evidence closure. This is what a user checks before trusting progress.
    box((1706, 520, 2338, 1126), "Active Work", ORANGE)
    work_rows = work_items[:4]
    y = 574
    for work in work_rows:
        issue = f"#{work.get('githubIssue', '—')}"
        title = work.get("title", "")
        lifecycle = work.get("lifecycle", "unknown")
        color = work_color(lifecycle)
        owner = "@host" if work.get("id") == active_work.get("id") else "@role"
        draw_text(draw, (1738, y), issue, BLUE, BOLD)
        draw_text(draw, (1824, y), fit_text(draw, title, 290, SMALL_BOLD), WHITE, SMALL_BOLD)
        draw_text(draw, (2168, y), fit_text(draw, lifecycle, 150, SMALL_BOLD), color, SMALL_BOLD)
        draw_text(draw, (2168, y + 32), owner, PURPLE if owner == "@host" else MUTED, SMALL)
        y += 82

    draw_text(draw, (1738, 930), "evidence closure", YELLOW, TITLE)
    active_evidence = next(
        (item for item in evidence_items if item.get("workOrder") == active_work.get("id")),
        evidence_items[0] if evidence_items else {},
    )
    closure_rows = [
        ("issue", f"#{active_work.get('githubIssue', '—')}", GREEN),
        ("branch", active_work.get("branch", "not-started"), CYAN),
        (
            "evidence",
            active_evidence.get("status", "missing"),
            evidence_color(active_evidence.get("status", "missing"), active_evidence.get("trackerUpdated", False)),
        ),
        (
            "tracker",
            "updated" if active_evidence.get("trackerUpdated") else "checkbox + ledger pending",
            GREEN if active_evidence.get("trackerUpdated") else ORANGE,
        ),
        ("rollback", active_evidence.get("rollback", "revert generated mock PR"), GREEN),
    ]
    y = 982
    for label, value, color in closure_rows:
        draw_text(draw, (1738, y), label, MUTED, SMALL)
        draw_text(draw, (1884, y), fit_text(draw, value, 405, SMALL_BOLD), color, SMALL_BOLD)
        y += 32

    # Bottom: gate pipeline and actionable alerts.
    box((58, 1152, 1668, 1238), "Gate Pipeline", GREEN)
    stages = [
        ("intake", GREEN),
        ("discovery", GREEN),
        ("plan", CYAN),
        ("review", ORANGE),
        ("signoff", DIM),
        ("implement", DIM),
        ("qa", DIM),
        ("closed", DIM),
    ]
    cursor = 96
    for label, color in stages:
        w = 150 if label in {"discovery", "implement"} else 118
        draw.rectangle((cursor, 1182, cursor + w, 1224), outline=color, width=2)
        draw_text(draw, (cursor + 14, 1189), label, color, SMALL_BOLD)
        cursor += w + 26
        if label != "closed":
            draw_text(draw, (cursor - 21, 1189), "→", MUTED, SMALL_BOLD)

    box((1706, 1152, 2338, 1238), "Alerts", RED)
    alert_x = 1738
    for alert in alerts[:3]:
        draw_text(draw, (alert_x, 1186), fit_text(draw, alert.get("title", ""), 185, SMALL_BOLD), RED if alert.get("severity") == "blocking" else ORANGE, SMALL_BOLD)
        alert_x += 205

    tabs = [
        ("<room>", True),
        ("<conversation>", False),
        ("<roles>", False),
        ("<workorders>", False),
        ("<gates>", False),
        ("<evidence>", False),
        ("<sources>", False),
        ("<logs>", False),
    ]
    x = 34
    for label, active in tabs:
        width = text_width(draw, label, SMALL_BOLD) + 34
        draw.rectangle((x, 1282, x + width, 1326), fill=CYAN if active else (24, 24, 24))
        draw_text(draw, (x + 17, 1290), label, BG if active else CYAN, SMALL_BOLD)
        x += width + 22
    draw_text(draw, (1580, 1292), "primary surface: talk to @host · dashboard shows live state", MUTED, SMALL)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    image.save(OUT_DIR / "control-room-console.png")


def render_work_cards() -> None:
    image, draw = new_canvas()
    prompt(draw, 82, 84, "@host finish v0.9 terminal QA and README visuals")
    status_line(draw, 134, "@host")
    active_card(
        draw,
        208,
        "@host",
        "drive v0.9 console QA gate",
        "collecting visual and tracker evidence",
        [
            ("✓", GREEN, "rendered 80 / 120 / 160 / 220 column console fixtures"),
            ("✓", GREEN, "checked public transcript and host action overlay clarity"),
            ("…", WHITE, "refreshing deterministic README images before release"),
        ],
        PURPLE,
    )

    permission_card(
        draw,
        430,
        "@qa",
        "manual visual verification",
        ORANGE,
    )

    done_summary(draw, 636, "@ci", "verify terminal fixtures", "1m12s", 4, CI)
    reply_quote(
        draw,
        684,
        "@ci",
        "@host",
        "console terminal QA fixtures and README image regeneration are reproducible",
        CI,
        PURPLE,
    )
    chat_line(
        draw,
        732,
        "@ci",
        "Evidence packet tracks issue #261, PR validation, images, and tracker closure.",
        CI,
    )

    done_summary(draw, 830, "@host", "drive v0.9 console QA gate", "2m41s", 9, PURPLE)
    right_rail(draw)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    image.save(OUT_DIR / "work-cards.png")


def main() -> int:
    render_boot_dashboard()
    render_work_cards()
    render_control_room_console()
    print("rendered docs/images/boot-dashboard.png")
    print("rendered docs/images/work-cards.png")
    print("rendered docs/images/control-room-console.png")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
