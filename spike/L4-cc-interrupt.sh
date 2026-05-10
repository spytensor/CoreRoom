#!/usr/bin/env bash
# L4 — probe whether Claude Code's stream-json stdin accepts an
# `interrupt` control message that aborts an in-flight tool call and
# emits a partial reply. v0.2 PR a's gemini and codex cancel paths land
# without depending on cc's verb (they use SIGTERM and JSON-RPC
# `notifications/cancelled` respectively); cc's path is gated on this
# probe per `docs/v0.2-trust-and-interrupt.md` § F.1.
#
# Pass: claude responds to `{"type":"interrupt"}` with either a partial
# `assistant`/`result` envelope OR a clean stream end (no panic, no
# stream-json schema violation).
#
# Fail: claude rejects the message as malformed input, closes stdin
# with no terminal envelope, or panics. PR b then falls back to the
# `stop_tx + respawn` cancellation strategy (same UX, higher cost).
#
# Cost: ~one short claude turn ($pennies; the prompt asks for a
# deliberately long-running ls so we have something to interrupt).
#
# How to run: `bash spike/L4-cc-interrupt.sh` from the repo root with
# `claude` on PATH and ANTHROPIC_API_KEY in the environment.

set -euo pipefail

if ! command -v claude >/dev/null 2>&1; then
    echo "claude binary not on PATH; aborting" >&2
    exit 2
fi
if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
    echo "ANTHROPIC_API_KEY unset; aborting" >&2
    exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

stdin_log="$work/stdin.jsonl"
stdout_log="$work/stdout.jsonl"

# Build a stdin script:
#   1. one user message asking for an iterative grep that takes a few seconds
#   2. an `interrupt` envelope sent ~1s later via a separate writer
#
# We can't easily delay between messages with a single heredoc; use a
# fifo and two writers.
fifo="$work/cc-stdin.fifo"
mkfifo "$fifo"

claude --print \
    --input-format stream-json \
    --output-format stream-json \
    --include-partial-messages \
    --dangerously-skip-permissions \
    < "$fifo" > "$stdout_log" 2>"$work/stderr.log" &
cc_pid=$!

# Keep fifo open for writes by exec'ing fd 9.
exec 9>"$fifo"

write_msg() {
    local payload=$1
    printf '%s\n' "$payload" >&9
    printf '%s\n' "$payload" >> "$stdin_log"
}

write_msg '{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Run `find / -name *.rs 2>/dev/null | head -200` then summarize."}]}}'

# Let the model start working before we interrupt.
sleep 2

write_msg '{"type":"interrupt"}'

# Close stdin so claude can drain and exit cleanly.
exec 9>&-

# Wait up to 30s for claude to terminate.
for _ in $(seq 1 30); do
    if ! kill -0 "$cc_pid" 2>/dev/null; then
        break
    fi
    sleep 1
done
if kill -0 "$cc_pid" 2>/dev/null; then
    echo "claude did not exit after interrupt — sending SIGTERM" >&2
    kill "$cc_pid" || true
    wait "$cc_pid" || true
    echo "VERDICT: FAIL — claude wedged on interrupt; v0.2 cc cancel path must use stop_tx + respawn" >&2
    exit 1
fi
wait "$cc_pid" || true

echo "--- stdout envelope types ---"
jq -r 'select(.type) | .type' < "$stdout_log" | sort | uniq -c

if grep -q '"subtype":"error"' "$stdout_log"; then
    echo "VERDICT: FAIL — claude reported an error envelope; cc cancel must fall back to respawn" >&2
    exit 1
fi

if ! grep -q '"type":"result"' "$stdout_log"; then
    echo "VERDICT: PARTIAL — no terminal result envelope; behavior unspecified" >&2
    exit 1
fi

echo "VERDICT: PASS — claude accepted {\"type\":\"interrupt\"} and returned a result envelope."
