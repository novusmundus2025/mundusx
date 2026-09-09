import unittest
from hermes_bridge import tool_activity, tool_outcome, AnswerStream, loaded_skill


class ProgressTests(unittest.TestCase):
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
        stream.delta("x" * 9000)
        self.assertEqual(len(events[-1]["data"]["text"]), 8192)
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
