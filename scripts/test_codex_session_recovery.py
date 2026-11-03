import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import unittest
import tempfile


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


class VisibleSessionRecoveryTests(unittest.TestCase):
    reply = ("Before failures are now gray and After failures remain red. "
             "The copy link control and the View Script action are also ready for review.")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = self.root / "transcript.jsonl"
        self.state = sqlite3.connect(":memory:")
        self.addCleanup(self.state.close)
        self.state.execute("CREATE TABLE threads "
                           "(id TEXT, name TEXT, title TEXT, archived INTEGER, cwd TEXT, rollout_path TEXT)")
        self.state.execute("INSERT INTO threads VALUES ('current', 'Review calls', '', 0, ?, ?)",
                           (str(self.root), str(self.path)))
        self.write_reply(self.reply)

    def write_reply(self, text, role="assistant", phase="final_answer", mode="w"):
        with self.path.open(mode) as out:
            out.write(json.dumps({"type": "response_item", "payload": {
                "type": "message", "role": role, "phase": phase,
                "content": [{"type": "output_text", "text": text}]}}) + "\n")

    def resolve(self, title="Review calls", checkout=None, text=None, excluded=None):
        return recovery.recover_visible(self.state, title, checkout or str(self.root),
                                        text or self.reply, excluded)

    def test_live_final_corroborates_same_checkout_session_switch_without_logs(self):
        self.assertEqual(self.resolve(text=self.reply.replace(" and ", "\n  and ")), "current")

    def test_title_checkout_and_reply_must_all_match(self):
        self.assertIsNone(self.resolve(title="Neighbor"))
        self.assertIsNone(self.resolve(checkout=str(self.root / "neighbor")))
        self.assertIsNone(self.resolve(text="Different live output"))

    def test_duplicate_name_in_same_checkout_is_ambiguous_even_with_matching_reply(self):
        self.state.execute("INSERT INTO threads SELECT 'neighbor',name,title,archived,cwd,rollout_path FROM threads")
        self.assertIsNone(self.resolve())

    def test_duplicate_name_in_other_checkout_is_also_ambiguous(self):
        self.state.execute("INSERT INTO threads SELECT 'fork',name,title,archived,?,rollout_path FROM threads",
                           (str(self.root / "worktree"),))
        self.assertIsNone(self.resolve())

    def test_neighbor_with_same_reply_does_not_match_by_content_alone(self):
        self.state.execute("UPDATE threads SET name='Neighbor'")
        self.assertIsNone(self.resolve())

    def test_clear_cannot_restore_old_session_even_while_reply_remains_visible(self):
        self.assertIsNone(self.resolve(excluded="current"))
        self.assertEqual(self.resolve(excluded="old"), "current")

    def test_only_latest_final_reply_qualifies(self):
        self.write_reply("A different final reply that is too short.", mode="a")
        self.assertIsNone(self.resolve())

    def test_user_quotes_and_commentary_do_not_qualify(self):
        for role, phase in [("user", "final_answer"), ("assistant", "commentary")]:
            self.write_reply(self.reply, role=role, phase=phase)
            self.assertIsNone(self.resolve())

    def test_short_generic_reply_does_not_qualify(self):
        self.write_reply("Done. Build passed.")
        self.assertIsNone(self.resolve(text="Done. Build passed."))

    def test_visible_recovery_survives_missing_logs_database(self):
        self.state.commit()
        with sqlite3.connect(self.root / "state_5.sqlite") as state:
            self.state.backup(state)
        result = subprocess.run(
            [sys.executable, str(ASSET)], input=json.dumps([{
                "pid": os.getpid(), "title": "Review calls", "checkout": str(self.root),
                "detection_text": self.reply,
            }]), text=True, capture_output=True, check=True,
            env={**os.environ, "CODEX_HOME": str(self.root)},
        )
        self.assertEqual(json.loads(result.stdout), [{"pid": os.getpid(), "session_id": "current"}])


if __name__ == "__main__":
    unittest.main()
