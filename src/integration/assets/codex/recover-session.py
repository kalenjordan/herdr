"""Reconcile Codex identity using process-owned logs or corroborated live output.

This is a fallback for shared app-server resumes that do not report a session
through the owning pane's hook. Missing or changed database schemas fail closed.
"""

import json
from contextlib import ExitStack
import os
import re
from pathlib import Path
import sqlite3
import subprocess
import sys
import time
from datetime import datetime


def read_only(path):
    return sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True, timeout=0.2)


def recover(logs, state, pid, started_at, title):
    # A title alone is not ownership evidence. Require a structured thread ID
    # emitted by this exact TUI process during its current lifetime as well.
    candidates = state.execute(
        "SELECT id, COALESCE(NULLIF(name, ''), title) FROM threads WHERE archived=0"
    )
    matches = []
    for session_id, name in candidates:
        if not name or not (title == name or title.startswith(name + " | ")):
            continue
        rows = logs.execute(
            "SELECT process_uuid, ts, feedback_log_body FROM logs "
            "WHERE thread_id=? AND ts>=? AND process_uuid GLOB ? "
            "AND target='codex_tui::app::history_pagination' "
            "ORDER BY ts DESC, ts_nanos DESC, id DESC LIMIT 1",
            (session_id, started_at, f"pid:{pid}:*"),
        ).fetchall()
        if not rows:
            continue
        process_uuid, _, body = rows[0]
        # History overlays can show another thread without making it active.
        if not body or not body.endswith("overlay=false"):
            continue
        startup = logs.execute(
            "SELECT ts FROM logs WHERE process_uuid=? AND thread_id IS NULL "
            "AND target='codex_tui::app::startup' AND ts>=? "
            "ORDER BY ts, ts_nanos, id LIMIT 1",
            (process_uuid, started_at),
        ).fetchone()
        # The generation UUID and launch timestamp exclude reused PIDs.
        if startup and 0 <= startup[0] - started_at <= 2:
            matches.append(session_id)
    return matches[0] if len(matches) == 1 else None


def normalized(text):
    return " ".join(re.sub(r"[`*_]", "", text).split())


def recover_visible(state, title, checkout, detection_text, excluded_session=None):
    # The caller validates this footer against the terminal's repository and
    # rechecks it and the process before applying the result. Never infer identity
    # from the directory, newest transcript, or a title alone.
    if not checkout or not detection_text:
        return None
    matches = list(state.execute(
        "SELECT id, cwd, rollout_path FROM threads "
        "WHERE archived=0 AND COALESCE(NULLIF(name, ''), title)=?", (title,)
    ))
    if len(matches) != 1 or matches[0][0] == excluded_session:
        return None
    session_id, _, path = matches[0]
    if Path(matches[0][1]).resolve() != Path(checkout).resolve():
        return None
    # Bound reads and skip a partial first record. Only the newest final reply
    # qualifies, never commentary, a user quote, or an older matching reply.
    with open(path, "rb") as transcript:
        transcript.seek(0, 2)
        start = max(0, transcript.tell() - 1024 * 1024)
        transcript.seek(start)
        if start:
            transcript.readline()
        lines = transcript.read().splitlines()
    for line in reversed(lines):
        record = json.loads(line)
        payload = record.get("payload", {})
        if (record.get("type") != "response_item" or payload.get("type") != "message"
                or payload.get("role") != "assistant" or payload.get("phase") != "final_answer"):
            continue
        text = "\n".join(part.get("text", "") for part in payload.get("content", [])
                         if part.get("type") == "output_text")
        visible = normalized(detection_text)
        paragraphs = [normalized(part) for part in text.split("\n\n")]
        if any(len(part) >= 80 and len(part.split()) >= 12 and part in visible
               for part in paragraphs):
            return session_id
        return None
    return None


def process_started_at(pid):
    output = subprocess.check_output(
        ["ps", "-p", str(pid), "-o", "lstart="],
        text=True, stderr=subprocess.DEVNULL, timeout=1,
    ).strip()
    return int(datetime.strptime(output, "%a %b %d %H:%M:%S %Y").timestamp())


def main():
    root = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex")
    targets = json.load(sys.stdin)
    results = []
    with ExitStack() as stack:
        state = stack.enter_context(read_only(root / "state_5.sqlite"))
        try:
            logs = stack.enter_context(read_only(root / "logs_2.sqlite"))
        except sqlite3.Error:
            logs = None
        # Bound database work; cancellation also prevents a growing log from
        # keeping a background recovery worker busy indefinitely.
        deadline = time.monotonic() + 2
        if logs is not None:
            logs.set_progress_handler(lambda: time.monotonic() >= deadline, 1000)
        state.set_progress_handler(lambda: time.monotonic() >= deadline, 1000)
        for target in targets:
            try:
                pid = target["pid"]
                started_at = process_started_at(pid)
                try:
                    session = recover(logs, state, pid, started_at, target["title"]) if logs else None
                except sqlite3.Error:
                    session = None
                if not session:
                    session = recover_visible(
                        state, target["title"], target.get("checkout"),
                        target.get("detection_text"), target.get("excluded_session"))
                if session == target.get("excluded_session"):
                    continue
                if session and process_started_at(pid) == started_at:
                    results.append({"pid": pid, "session_id": session})
            except (OSError, ValueError, subprocess.SubprocessError, sqlite3.Error):
                continue
    print(json.dumps(results))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, sqlite3.Error):
        print("[]")
