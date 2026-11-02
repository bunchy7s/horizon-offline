import tempfile
import unittest
import pathlib
from tools import startup_trace


class StartupTraceTests(unittest.TestCase):
    def test_steps_and_marks_from_the_games_lines(self):
        lines = [
            '{"ph":"M","name":"thread_name","pid":1,"tid":1,"args":{"name":"main"}}',
            '{"pid":1,"tid":1,"ph":"X","name":"renderer.boot","ts":20000,"dur":150000}',
            '{"ph":"M","name":"thread_name","pid":1,"tid":2,"args":{"name":"world-loader"}}',
            '{"pid":1,"tid":2,"ph":"X","name":"ui.load","ts":5000,"dur":400000}',
            '{"pid":1,"tid":1,"ph":"i","s":"g","name":"title.shown","ts":450000}',
            '{"pid":1,"tid":1,"ph":"i","s":"g","name":"title.shown","ts":900000}',
            '{"pid":1,"tid":1,"ph":"X","name":"torn',
        ]
        with tempfile.TemporaryDirectory() as d:
            path = pathlib.Path(d) / "trace.jsonl"
            path.write_text("\n".join(lines))
            steps, marks = startup_trace.summarize(startup_trace.read_trace(path))
        self.assertEqual(steps, [(5.0, 405.0, "ui.load", "world-loader"),
                                 (20.0, 170.0, "renderer.boot", "main")])
        # The first time a mark happened.
        self.assertEqual(marks, {"title.shown": 450.0})

    def test_a_missing_trace_has_no_events(self):
        self.assertEqual(startup_trace.read_trace("/nonexistent/trace.jsonl"), [])


if __name__ == "__main__":
    unittest.main()
