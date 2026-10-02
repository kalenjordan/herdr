"""Read process-owned Codex TUI evidence without reading conversation content.

This is a fallback for shared app-server resumes that do not report a session
through the owning pane's hook. Missing or changed database schemas fail closed.
"""

import json
import os
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
    with read_only(root / "logs_2.sqlite") as logs, read_only(root / "state_5.sqlite") as state:
        # Bound database work; cancellation also prevents a growing log from
        # keeping a background recovery worker busy indefinitely.
        deadline = time.monotonic() + 2
        logs.set_progress_handler(lambda: time.monotonic() >= deadline, 1000)
        state.set_progress_handler(lambda: time.monotonic() >= deadline, 1000)
        for target in targets:
            try:
                pid = target["pid"]
                started_at = process_started_at(pid)
                session = recover(logs, state, pid, started_at, target["title"])
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
