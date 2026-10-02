import importlib.util
from pathlib import Path
import sqlite3
import unittest


ASSET = Path(__file__).resolve().parents[1] / "src/integration/assets/codex/recover-session.py"
spec = importlib.util.spec_from_file_location("codex_recovery", ASSET)
recovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recovery)


class CodexSessionRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.logs = sqlite3.connect(":memory:")
        self.state = sqlite3.connect(":memory:")
        self.addCleanup(self.logs.close)
        self.addCleanup(self.state.close)
        self.logs.execute("CREATE TABLE logs (id INTEGER PRIMARY KEY, process_uuid TEXT, "
                          "ts INTEGER, ts_nanos INTEGER, thread_id TEXT, target TEXT, feedback_log_body TEXT)")
        self.state.execute("CREATE TABLE threads (id TEXT, name TEXT, title TEXT, archived INTEGER)")
        self.state.execute("INSERT INTO threads VALUES ('old-session', 'Analyze calls', 'Original prompt', 0)")
        self.evidence("old-session", "pid:42:current", 100, 120)

    def evidence(self, session, process, startup, observed, overlay=False):
        self.logs.execute("INSERT INTO logs VALUES (NULL, ?, ?, 0, NULL, "
                          "'codex_tui::app::startup', 'tui startup initial frame scheduled')", (process, startup))
        self.logs.execute("INSERT INTO logs VALUES (NULL, ?, ?, 0, ?, "
                          "'codex_tui::app::history_pagination', ?)",
                          (process, observed, session, f"loading history overlay={str(overlay).lower()}"))

    def resolve(self, pid=42, started=100, title="Analyze calls | outbound-dash"):
        return recovery.recover(self.logs, self.state, pid, started, title)

    def test_resumed_session_matches_current_process_without_creation_time_or_cwd(self):
        # The session predates this process and its checkout differs from the
        # shell. Neither creation time nor shell cwd is ownership evidence.
        self.assertEqual(self.resolve(), "old-session")

    def test_wrong_process_and_reused_pid_do_not_match(self):
        self.assertIsNone(self.resolve(pid=43))
        self.assertIsNone(self.resolve(started=121))
        self.assertIsNone(self.resolve(started=90))

    def test_current_terminal_title_is_required(self):
        self.assertIsNone(self.resolve(title="Different session | outbound-dash"))
        self.assertIsNone(self.resolve(title="Analyze calls extended | outbound-dash"))

    def test_history_overlay_is_not_active_session_evidence(self):
        self.logs.execute("UPDATE logs SET feedback_log_body='loading history overlay=true' "
                          "WHERE thread_id IS NOT NULL")
        self.assertIsNone(self.resolve())

    def test_ambiguous_session_names_are_rejected(self):
        self.state.execute("INSERT INTO threads VALUES ('other-session', 'Analyze calls', '', 0)")
        self.evidence("other-session", "pid:42:current", 100, 130)
        self.assertIsNone(self.resolve())

    def test_logs_from_other_process_generations_are_rejected(self):
        self.logs.execute("DELETE FROM logs WHERE thread_id IS NULL")
        self.evidence("other-session", "pid:42:previous", 50, 60)
        self.assertIsNone(self.resolve())

    def test_archived_session_is_not_recovered(self):
        self.state.execute("UPDATE threads SET archived=1")
        self.assertIsNone(self.resolve())

    def test_session_without_name_uses_exact_title(self):
        self.state.execute("UPDATE threads SET name=NULL, title='Analyze calls'")
        self.assertEqual(self.resolve(), "old-session")


if __name__ == "__main__":
    unittest.main()
