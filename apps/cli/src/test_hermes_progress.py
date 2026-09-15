import json
import tempfile
import unittest
from hermes_bridge import (
    AnswerStream,
    EXECUTION_EFFICIENCY_GUIDANCE,
    FRONTEND_MAX_ITERATIONS,
    PROJECT_COMPACTION_TOKENS,
    RecoveryCheckpoint,
    STANDARD_MAX_ITERATIONS,
    browser_verification,
    configure_project_compaction,
    loaded_skill,
    tool_activity,
    tool_outcome,
)


class ProgressTests(unittest.TestCase):
    def test_recovery_checkpoint_is_bounded_atomic_and_recovery_only(self):
        with tempfile.TemporaryDirectory() as root:
            checkpoint = RecoveryCheckpoint(root, "task/unsafe")
            self.assertEqual(checkpoint.recovery_note(), "")
            checkpoint.start()
            checkpoint.record("write_file", {"path": "src/app.js"}, {"bytes_written": 12})
            resumed = RecoveryCheckpoint(root, "task/unsafe")
            note = resumed.recovery_note()
            self.assertIn("src/app.js", note)
            self.assertNotIn("bytes_written", note)
            resumed.events = [{"tool": str(index)} for index in range(80)]
            resumed.finish(True)
            with open(resumed.path, "r", encoding="utf-8") as handle:
                payload = json.load(handle)
            self.assertEqual(payload["state"], "completed")
            self.assertEqual(len(payload["events"]), 48)
            self.assertEqual(RecoveryCheckpoint(root, "task/unsafe").recovery_note(), "")

    def test_project_turns_are_bounded_and_escape_aware(self):
        self.assertEqual(STANDARD_MAX_ITERATIONS, 12)
        self.assertEqual(FRONTEND_MAX_ITERATIONS, 16)
        self.assertIn("Do not treat that display escaping", EXECUTION_EFFICIENCY_GUIDANCE)
        self.assertIn("return the final response immediately", EXECUTION_EFFICIENCY_GUIDANCE)

    def test_project_compaction_uses_token_preflight_cap(self):
        class Compressor:
            threshold_tokens = 48_000
            threshold_tokens_cap = None

        class Agent:
            context_compressor = Compressor()

        self.assertTrue(configure_project_compaction(Agent()))
        self.assertEqual(Agent.context_compressor.threshold_tokens_cap, PROJECT_COMPACTION_TOKENS)
        self.assertEqual(Agent.context_compressor.threshold_tokens, PROJECT_COMPACTION_TOKENS)
        self.assertFalse(configure_project_compaction(object()))

    def test_skill_progress_requires_actual_success(self):
        self.assertEqual(loaded_skill("skill_view", '{"success":true,"name":"debugging"}'), "debugging")
        for result in [{"success": False, "name": "debugging"}, {"name": "debugging"}, {"success": True, "name": "debugging", "error": "failed"}, "bad json"]:
            self.assertIsNone(loaded_skill("skill_view", result))
        self.assertIsNone(loaded_skill("skills_list", {"success": True, "name": "debugging"}))
        self.assertEqual(tool_activity("skills_list", {}), "skill_discovery")
        self.assertEqual(tool_activity("skill_view", {}), "skill_load")

    def test_answer_stream_batches_and_flushes_complete_snapshots(self):
        events, now = [], [1.0]
        stream = AnswerStream(events.append, lambda: now[0])
        stream.delta("Hello")
        stream.delta(" world")
        self.assertEqual(len(events), 1)
        stream.flush()
        self.assertEqual(events[-1]["data"]["text"], "Hello world")
        stream.flush()
        self.assertEqual(len(events), 2)
        now[0] += 1
        stream.delta("x" * 40000)
        self.assertEqual(len(events[-1]["data"]["text"]), 32768)
        self.assertTrue(events[-1]["data"]["truncated"])

    def test_activity_is_observed_without_copying_arguments(self):
        for command, activity in [('echo "5" | node "C:/project/menu.js"', "run"), ("npm run build", "build"), ("npm test", "test"), ("npm run lint", "lint"), ("echo npm test", "command")]:
            self.assertEqual(tool_activity("terminal", {"command": command}), activity)
        self.assertEqual(tool_activity("terminal", {"command": "curl --token SECRET"}), "command")
        self.assertEqual(tool_activity("terminal", []), "command")

    def test_outcome_does_not_invent_success(self):
        self.assertIs(tool_outcome({"exit_code": 0}), True)
        self.assertIs(tool_outcome({"exit_code": 2}), False)
        self.assertIs(tool_outcome({"success": False}), False)
        self.assertIs(tool_outcome('{"exit_code": 1}'), False)
        self.assertIsNone(tool_outcome("Unstructured command output"))

    def test_browser_tools_report_real_success(self):
        self.assertIs(tool_outcome('{"success":true,"title":"Enrollment System"}', "browser_navigate"), True)
        self.assertIs(tool_outcome('{"success":false,"error":"page crashed"}', "browser_console"), False)
        self.assertEqual(browser_verification("browser_exec", {"success": True}), "suite")
        self.assertIsNone(browser_verification("browser_exec", {"success": False}))

    def test_hermes_file_results(self):
        self.assertIs(tool_outcome({"content": "", "total_lines": 0}, "read_file"), True)
        self.assertIs(tool_outcome({"bytes_written": 0}, "write_file"), True)
        self.assertIs(tool_outcome({"total_count": 0}, "search_files"), True)
        self.assertIs(tool_outcome({"bytes_written": 0, "error": "Permission denied"}, "write_file"), False)
        self.assertIs(tool_outcome({"success": True, "error": "Partial failure"}), False)
        self.assertIsNone(tool_outcome({"content": "some text"}, "terminal"))
        self.assertIsNone(tool_outcome({"bytes_written": True}, "write_file"))


if __name__ == "__main__":
    unittest.main()
