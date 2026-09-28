#!/bin/sh
# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# HERDR_INTEGRATION_ID=codex
# HERDR_INTEGRATION_VERSION=7

set -eu

action="${1:-}"
hook_input_file="$(mktemp "${TMPDIR:-/tmp}/herdr-codex-hook.XXXXXX")" || exit 0
trap 'rm -f "$hook_input_file"' EXIT HUP INT TERM
cat >"$hook_input_file" 2>/dev/null || true

case "$action" in
  session) ;;
  *) exit 0 ;;
esac

[ "${HERDR_ENV:-}" = "1" ] || exit 0
[ -n "${HERDR_SOCKET_PATH:-}" ] || exit 0
[ -n "${HERDR_PANE_ID:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

HERDR_ACTION="$action" HERDR_HOOK_INPUT_FILE="$hook_input_file" python3 - <<'PY'
import json
import os
import random
import re
import socket
import subprocess
import time
from datetime import datetime
from pathlib import Path

source = "herdr:codex"
action = os.environ.get("HERDR_ACTION", "")
pane_id = os.environ.get("HERDR_PANE_ID")
socket_path = os.environ.get("HERDR_SOCKET_PATH")
hook_input_file = os.environ.get("HERDR_HOOK_INPUT_FILE")

if not pane_id or not socket_path:
    raise SystemExit(0)

hook_input = {}
if hook_input_file:
    try:
        with open(hook_input_file, encoding="utf-8") as handle:
            content = handle.read()
        if content.strip():
            hook_input = json.loads(content)
    except Exception:
        hook_input = {}

hook_event_name = str(hook_input.get("hook_event_name") or "")
if hook_event_name and hook_event_name != "SessionStart":
    raise SystemExit(0)

report_seq = time.time_ns()
session_id = hook_input.get("session_id")
agent_session_id = session_id if isinstance(session_id, str) and session_id else None
session_start_source = hook_input.get("source") if hook_event_name == "SessionStart" else None
if not isinstance(session_start_source, str) or not session_start_source:
    session_start_source = None
if agent_session_id:
    params = {
        "pane_id": pane_id,
        "source": source,
        "agent": "codex",
        "seq": report_seq,
        "agent_session_id": agent_session_id,
    }
    if session_start_source:
        params["session_start_source"] = session_start_source
else:
    raise SystemExit(0)

def request(method, params):
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.settimeout(0.5)
            client.connect(socket_path)
            request_id = f"{source}:{int(time.time() * 1000)}:{random.randrange(1_000_000):06d}"
            client.sendall((json.dumps({"id": request_id, "method": method, "params": params}) + "\n").encode())
            with client.makefile("r", encoding="utf-8") as response:
                return json.loads(response.readline())
    except Exception:
        return None

def process_started_at(pid):
    try:
        output = subprocess.check_output(
            ["ps", "-p", str(pid), "-o", "lstart="],
            text=True, stderr=subprocess.DEVNULL, timeout=0.5,
        ).strip()
        return datetime.strptime(output, "%a %b %d %H:%M:%S %Y")
    except Exception:
        return None

def transcript_started_at():
    path = hook_input.get("transcript_path")
    if not isinstance(path, str) or not path:
        if not re.fullmatch(r"[0-9a-fA-F-]{36}", agent_session_id):
            return None
        root = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex") / "sessions"
        found = list(root.glob(f"*/*/*/*{agent_session_id}.jsonl"))
        if len(found) != 1:
            return None
        path = str(found[0])
    match = re.match(r"rollout-(\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2})-", Path(path).name)
    if not match:
        return None
    try:
        return datetime.strptime(match.group(1), "%Y-%m-%dT%H-%M-%S")
    except ValueError:
        return None

def live_pane_id():
    listed = request("pane.list", {})
    panes = (listed or {}).get("result", {}).get("panes", [])
    if not isinstance(panes, list):
        return None
    existing = [pane.get("pane_id") for pane in panes if isinstance(pane, dict)
                and (pane.get("agent_session") or {}).get("kind") == "id"
                and (pane.get("agent_session") or {}).get("value") == agent_session_id]
    if len(existing) == 1:
        return existing[0]
    cwd = hook_input.get("cwd") or os.getcwd()
    if not isinstance(cwd, str) or not cwd:
        return None
    candidates = [pane for pane in panes if isinstance(pane, dict)
                  and (pane.get("cwd") == cwd or pane.get("foreground_cwd") == cwd)
                  and pane.get("agent") == "codex"]
    started_at = transcript_started_at()
    matches = []
    if started_at:
        for pane in candidates:
            info = request("pane.process_info", {"pane_id": pane.get("pane_id")})
            processes = (info or {}).get("result", {}).get("process_info", {}).get("foreground_processes", [])
            if any(process.get("name") == "codex" and
                   (actual := process_started_at(process.get("pid"))) is not None and
                   abs((actual - started_at).total_seconds()) <= 5
                   for process in processes if isinstance(process, dict)):
                matches.append(pane.get("pane_id"))
    if len(matches) == 1:
        return matches[0]
    if session_start_source == "clear":
        # /clear keeps the Codex process alive. Codex's app server may run
        # hooks with a pane ID inherited from a different tab.
        focused = [pane for pane in candidates if pane.get("focused") is True]
        if len(focused) == 1:
            tab_id = focused[0].get("tab_id")
            tab = request("tab.get", {"tab_id": tab_id}) if isinstance(tab_id, str) else None
            label = (tab or {}).get("result", {}).get("tab", {}).get("label")
            if isinstance(label, str) and label.isdigit():
                return focused[0].get("pane_id")
    unclaimed = [pane.get("pane_id") for pane in candidates if not pane.get("agent_session")]
    return unclaimed[0] if len(candidates) == 1 and len(unclaimed) == 1 else None

resolved_pane_id = live_pane_id()
if resolved_pane_id:
    params["pane_id"] = resolved_pane_id
    request("pane.report_agent_session", params)
PY
