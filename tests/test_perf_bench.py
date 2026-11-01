import json
import sys
import unittest

from tools.perf_bench import HISTOGRAM_EDGES_MS, OPTIONS, frames, rnd, summary


class ComparisonConditionsTests(unittest.TestCase):
    def test_rejects_incompatible_or_unknown_conditions_before_printing(self):
        import contextlib
        import io
        import tempfile
        from pathlib import Path
        from unittest import mock
        import tools.perf_bench as pb

        conditions = {"scenario": "flight", "seed": "1234", "render_distance": 8, "simulation_distance": 12,
                      "window": [1920, 1080], "vsync": False, "offscreen": False}
        differences = {"scenario": "other", "seed": "5678", "simulation_distance": 8, "window": [1280, 720],
                       "vsync": True, "offscreen": True, "fps_cap": 60}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            reports = root / "benchmarks/perf"
            reports.mkdir(parents=True)
            def write(label, values):
                (reports / f"{label}-rd8.json").write_text(
                    json.dumps({"conditions": values, "metrics": {}}), encoding="utf-8")
            write("first", conditions)
            with mock.patch.object(pb, "ROOT", root):
                for key, value in differences.items():
                    with self.subTest(key=key):
                        write("second", dict(conditions, **{key: value}))
                        out = io.StringIO()
                        with contextlib.redirect_stdout(out), self.assertRaisesRegex(ValueError, key):
                            pb.table(["first", "second"])
                        self.assertEqual(out.getvalue(), "")
                incomplete = dict(conditions)
                del incomplete["window"]
                write("second", incomplete)
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(ValueError, "window"):
                    pb.table(["first", "second"])
                stderr, stdout = io.StringIO(), io.StringIO()
                with mock.patch("sys.argv", ["perf_bench.py", "--table", "first", "second"]), \
                        contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr), \
                        self.assertRaises(SystemExit) as exit_error:
                    pb.main()
                self.assertEqual(exit_error.exception.code, 2)
                self.assertEqual(stdout.getvalue(), "")
                self.assertIn("window missing in second", stderr.getvalue())
                self.assertNotIn("Traceback", stderr.getvalue())
                write("second", dict(conditions, fps_cap=None))
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(ValueError, "fps_cap"):
                    pb.table(["first", "second"])

    def test_matching_reports_and_single_incomplete_report_remain_readable(self):
        import contextlib
        import io
        import tempfile
        from pathlib import Path
        from unittest import mock
        import tools.perf_bench as pb

        conditions = {"scenario": "flight", "seed": "1234", "render_distance": 8, "simulation_distance": 12,
                      "window": [1920, 1080], "vsync": False, "offscreen": False}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            reports = root / "benchmarks/perf"
            reports.mkdir(parents=True)
            for label, values in (("first", conditions), ("second", conditions),
                                  ("single", {"render_distance": 8})):
                (reports / f"{label}-rd8.json").write_text(
                    json.dumps({"conditions": values, "metrics": {}}), encoding="utf-8")
            with mock.patch.object(pb, "ROOT", root), contextlib.redirect_stdout(io.StringIO()) as out:
                pb.table(["first", "second"])
                pb.table(["single"])
            self.assertIn("| 8 |", out.getvalue())



class PerfBenchStatsTests(unittest.TestCase):
    def test_lows_average_the_slowest_frames(self):
        times = [10.0] * 990 + [100.0] * 9 + [1000.0]
        s = frames("village", 0, times)
        self.assertEqual(s["frames"], 1000)
        self.assertEqual(s["avg_fps"], rnd(1000 * 1000 / (9900 + 900 + 1000)))
        # Slowest 1%: 9 frames of 100 ms and one of 1000 ms.
        self.assertEqual(s["low_1pct_fps"], rnd(1000 * 10 / 1900))
        # Slowest 0.1%: the single 1000 ms frame.
        self.assertEqual(s["low_01pct_fps"], 1.0)
        self.assertEqual((s["p50_ms"], s["p99_ms"], s["p999_ms"], s["max_ms"]), (10.0, 10.0, 100.0, 1000.0))
        self.assertEqual(sum(s["histogram"]), 1000)
        self.assertEqual(len(s["histogram"]), len(HISTOGRAM_EDGES_MS))
        self.assertEqual(s["histogram"][HISTOGRAM_EDGES_MS.index(10)], 990)
        self.assertEqual(s["histogram"][HISTOGRAM_EDGES_MS.index(100)], 9)
        self.assertEqual(s["histogram"][-1], 1, "1000 ms and over")

    def test_same_figures_as_horizon_bench(self):
        # crates/horizon-bench's frame_figures_and_histogram test, value for value.
        s = frames("flight", 0, [2.0] * 998 + [20.0, 1500.0])
        self.assertEqual((s["seconds"], s["low_1pct_fps"], s["low_01pct_fps"]), (3.516, 6.51, 0.667))
        self.assertEqual((s["histogram"][2], s["histogram"][13], s["histogram"][23]), (998, 1, 1))
        self.assertNotIn("cpu_p50_ms", s)
        r = summary([2.0, 4.0, 6.0])
        self.assertEqual(r, {"n": 3, "mean": 4.0, "stdev": 2.0, "min": 2.0, "max": 6.0, "ci95": 4.969})
        self.assertEqual(summary([5.0])["ci95"], None)
        self.assertIsNone(summary([None]))

    def test_a_short_flight_still_has_lows(self):
        s = frames("x", 0, [5.0, 20.0])
        self.assertEqual((s["low_1pct_fps"], s["low_01pct_fps"]), (50.0, 50.0))
        self.assertIsNone(frames("x", 0, []))
        json.dumps(s)

    def test_window_sizes_name_their_reports(self):
        from tools.perf_bench import SIZES, parse_sizes, size_label
        self.assertEqual(parse_sizes("1280x720,1920X1080"), [(1280, 720), (1920, 1080)])
        self.assertEqual(SIZES, [(1280, 720), (1920, 1080)])
        self.assertEqual(size_label("horizon-m4", 1280, 720), "horizon-m4")
        self.assertEqual(size_label("horizon-m4", 1920, 1080), "horizon-m4-1080p")
        self.assertEqual(size_label("horizon-m4", 2560, 1080), "horizon-m4-2560x1080")

    def test_one_options_file_sets_the_render_distance(self):
        self.assertEqual(OPTIONS["renderDistance"].format(distance=16), "16")
        self.assertEqual(OPTIONS["graphicsPreset"], '"custom"')

    def test_table_distinguishes_missing_scene_load_from_zero(self):
        import contextlib
        import io
        import tempfile
        from pathlib import Path
        from unittest import mock
        import tools.perf_bench as pb

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            reports = root / "benchmarks" / "perf"
            reports.mkdir(parents=True)
            # Comparable runs: table() refuses reports whose conditions differ or are missing.
            conditions = {"scenario": "flight", "seed": "1234", "render_distance": 8, "simulation_distance": 12,
                          "window": [1920, 1080], "vsync": False, "offscreen": False}
            for label, metrics in (
                ("missing", {}),
                ("zero", {"chunks.village.load_ms": {"mean": 0}}),
                ("measured", {"chunks.village.load_ms": {"mean": 1000},
                              "chunks.forest.load_ms": {"mean": 3000}}),
            ):
                (reports / f"{label}-rd8.json").write_text(
                    json.dumps({"conditions": conditions, "metrics": metrics}), encoding="utf-8")
            output = io.StringIO()
            with mock.patch.object(pb, "ROOT", root), contextlib.redirect_stdout(output):
                pb.table(["missing", "zero", "measured"])
        row = next(line for line in output.getvalue().splitlines() if line.startswith("| 8 |"))
        self.assertEqual([cell.strip().split(", ")[1] for cell in row.split("|")[2:-1]], ["–", "0.0", "2.0"])




if __name__ == "__main__":
    unittest.main()


class WaitLoadedTests(unittest.TestCase):
    """wait_loaded on a scripted game with a fake clock: one status per 50 ms poll."""

    def run_script(self, statuses, min_chunks=0):
        import tools.perf_bench as pb
        clock = [0.0]
        it = iter(statuses)

        def control(_command, timeout=None):
            return next(it)

        real_time, real_sleep = pb.time.time, pb.time.sleep
        pb.time.time = lambda: clock[0]
        pb.time.sleep = lambda s: clock.__setitem__(0, clock[0] + s)
        try:
            return pb.wait_loaded(control, lambda p: True, 60, min_chunks)
        finally:
            pb.time.time, pb.time.sleep = real_time, real_sleep

    @staticmethod
    def status(chunks, meshed, stages=None):
        s = {"player": {"position": [0, 0, 0]}, "pending_chunks": 0, "screen": "game",
             "chunks": chunks, "generated_chunks": chunks, "meshed_sections": meshed}
        if stages is not None:
            s["streaming"] = {"queued": 0, "generating": 0, "protos": 296, "decorating": 0,
                              "finishing": 0, "lighting": 0, "meshing": stages}
        return s

    def test_stages_end_the_load_before_the_remesh_tail(self):
        # 20 polls (1 s) of meshing work, then idle stages while blocks keep remeshing
        # at 400 meshes/s, with a short burst of work every 2 s.
        loading = [self.status(900, 1000 * i, stages=500) for i in range(20)]
        tail = [self.status(901, 20000 + 20 * i, stages=40 if i % 40 == 0 else 0) for i in range(400)]
        self.assertAlmostEqual(self.run_script(loading + tail), 1.0, places=6)

    def test_old_builds_wait_for_meshing_to_go_quiet(self):
        busy = [self.status(901, 1000 * i) for i in range(20)]
        quiet = [self.status(901, 20000 + i) for i in range(40)]
        self.assertAlmostEqual(self.run_script(busy + quiet), 1.0, places=6)

    def test_a_quiet_start_is_not_a_load(self):
        # Right after a teleport Horizon reports steady counts and nothing queued for 1.5 s before it loads:
        # with a target the clock runs on until the chunks are there and the meshing has gone quiet.
        idle = [self.status(3, 0, stages=0) for _ in range(30)]
        loading = [self.status(30 * (i + 1), 500 * i, stages=500) for i in range(20)]
        tail = [self.status(600, 10000, stages=0) for _ in range(60)]
        took = self.run_script(idle + loading + tail, min_chunks=500)
        self.assertGreater(took, 2.4)
        self.assertLess(took, 4.0)
        # Without one (the old call), the quiet start ends the wait at once.
        self.assertLess(self.run_script(idle + loading + tail), 1.6)


class PortabilityTests(unittest.TestCase):
    """What the driver needs from the OS also works where it differs (Windows: no ps, no load average,
    no Unix sockets in Python, nvidia-smi without per-process memory)."""

    def test_resident_memory_of_this_process(self):
        import os
        from tools.perf_bench import rss_bytes
        self.assertGreater(rss_bytes(os.getpid()), 1 << 20)

    def test_load_average_is_a_number_or_absent(self):
        from tools.perf_bench import load_average
        self.assertTrue(load_average() is None or load_average() >= 0)

    def test_report_without_a_load_average(self):
        import argparse
        import tools.perf_bench as pb
        run = {"startup": {"title_s": 1.0, "world_s": 5.0}, "memory": {}, "load_before": None,
               "scenes": {"village": {"load_s": 2.0, "frame_ms": [5.0] * 100, "meshes_load": 10}}}
        args = argparse.Namespace(offscreen=False, settle=2.0, label="x")
        r = pb.report(args, 8, [run], {"name": "horizon-client"})
        self.assertNotIn("load_avg_1m", r["conditions"])
        self.assertEqual(r["metrics"]["fps.village.avg_fps"]["mean"], 200.0)
        self.assertEqual(pb.report(args, 8, [dict(run, load_before=1.25)], {})["conditions"]["load_avg_1m"], 1.25)
        self.assertTrue(r["hardware"]["os"] and r["hardware"]["cpu_threads"])

    def test_nvidia_memory_not_available(self):
        from unittest import mock
        import subprocess
        import tools.perf_bench as pb
        out = subprocess.CompletedProcess([], 0, stdout="1234, [N/A]\n4321, 812\n")
        with mock.patch.object(pb.shutil, "which", return_value="nvidia-smi"), \
                mock.patch.object(pb.subprocess, "run", return_value=out):
            self.assertIsNone(pb.nvidia_memory_mb(1234))
            self.assertEqual(pb.nvidia_memory_mb(4321), 812.0)
            self.assertIsNone(pb.nvidia_memory_mb(99))

    @unittest.skipUnless(sys.platform == "win32", "the console's OEM code page is Windows'")
    def test_windows_version_reads_the_console_code_page(self):
        # pr_ready runs the tests with PYTHONUTF8=1, and `cmd /C ver` answers in the OEM code page:
        # decoding it as UTF-8 left `stdout` None (issue #5201).
        import os
        import subprocess
        from pathlib import Path
        root = Path(__file__).resolve().parents[1]
        out = subprocess.run(
            [sys.executable, "-c",
             "from tools.perf_bench import windows_version; print(windows_version())"],
            capture_output=True, text=True, encoding="utf-8", errors="replace", check=True,
            env={**os.environ, "PYTHONUTF8": "1"}, cwd=root)
        self.assertIn("Windows", out.stdout)

    def test_control_over_the_tcp_endpoint(self):
        # The game's TCP transport (Windows, or an endpoint path ending in .json): the file names a port and
        # a token, and the first line must be `auth TOKEN`.
        import socket
        import tempfile
        import threading
        from pathlib import Path
        from tools.perf_bench import Control, control_path
        self.assertIn(control_path().suffix, (".sock", ".json"))
        server = socket.create_server(("127.0.0.1", 0))
        lines = []

        def serve():
            for _ in range(2):
                conn, _ = server.accept()
                with conn, conn.makefile("rwb") as f:
                    auth = f.readline().decode().strip()
                    lines.append(auth)
                    if auth != "auth secret":
                        f.write(b'{"error":"bad or missing token"}\n')
                        f.flush()
                        continue
                    f.write(b'{"ok":true}\n')
                    f.flush()
                    lines.append(f.readline().decode().strip())
                    f.write(b'{"screen":"title"}\n')
                    f.flush()

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        with tempfile.TemporaryDirectory() as tmp, server:
            path = Path(tmp) / "control.json"
            port = server.getsockname()[1]
            path.write_text(json.dumps({"transport": "tcp", "port": port, "token": "secret"}))
            self.assertEqual(Control(path)("status", timeout=10), {"screen": "title"})
            path.write_text(json.dumps({"transport": "tcp", "port": port, "token": "wrong"}))
            with self.assertRaises(OSError):
                Control(path)("status", timeout=10)
            thread.join(10)
        self.assertEqual(lines, ["auth secret", "status", "auth wrong"])

    def test_a_release_build_reports_its_own_version(self):
        import tempfile
        from pathlib import Path
        from tools.perf_bench import release_version
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "horizon-rd8.log"
            log.write_text("Horizon v0.2.97\nload assets: 1.29s\n")
            self.assertEqual(release_version(log), "v0.2.97")
            log.write_text("Horizon nightly-2026-10-28-0c02067\n")
            self.assertEqual(release_version(log), "nightly-2026-10-28-0c02067")
            for dev in ("Horizon 0.1.0\n", "Horizon v0.2.97 (client 0.1.0)\n"):
                log.write_text(dev)
                self.assertIsNone(release_version(log))
            self.assertIsNone(release_version(Path(tmp) / "missing.log"))

    def test_vanilla_gets_the_manifest_jvm_flags(self):
        from tools.perf_bench import manifest_jvm_flags
        manifest = {"arguments": {"jvm": [
            {"rules": [{"action": "allow", "os": {"name": "osx"}}], "value": ["-XstartOnFirstThread"]},
            {"rules": [{"action": "allow", "os": {"arch": "x86"}}], "value": "-Xss1M"},
            "-XX:StackShadowPages=32", "--enable-native-access=ALL-UNNAMED",
            "-Djava.library.path=${natives_directory}/java", "-cp", "${classpath}"]}}
        self.assertEqual(manifest_jvm_flags(manifest), ["-XX:StackShadowPages=32"])
        self.assertEqual(manifest_jvm_flags({}), [])
