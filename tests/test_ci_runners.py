"""playhorizon's checks runners (docs/ci-runners.md) never run a fork's pull request."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
LABELS = ("playhorizon-ci", "playhorizon-light")
FORK_GUARD = "github.event.pull_request.head.repo.full_name == github.repository"


def jobs(text):
    """(name, block) for each job of a workflow, by indentation (two spaces under `jobs:`)."""
    body = text.split("\njobs:\n", 1)[1]
    parts = re.split(r"^  ([A-Za-z0-9_-]+):\n", body, flags=re.M)
    return list(zip(parts[1::2], parts[2::2]))


def triggers(text):
    head = text.split("\njobs:\n", 1)[0]
    on = re.search(r"^on:\n((?:[ #].*\n|\n)*)", head, re.M)
    return set(re.findall(r"^  ([a-z_]+):", on.group(1), re.M)) if on else set()


class ChecksRunnerTests(unittest.TestCase):
    def test_self_hosted_jobs_never_take_a_fork_pull_request(self):
        routed = 0
        for path in sorted(WORKFLOWS.glob("*.yml")):
            text = path.read_text()
            on = triggers(text)
            for name, block in jobs(text):
                runs_on = next((l for l in block.splitlines() if l.strip().startswith("runs-on:")), "")
                if not any(label in runs_on for label in LABELS):
                    continue
                routed += 1
                where = f"{path.name} {name}"
                # The switch back to GitHub's runners.
                self.assertTrue("vars.SELF_HOSTED_CI != 'off'" in runs_on or "vars.SELF_HOSTED_CI == 'all'" in runs_on,
                                where)
                self.assertIn("'ubuntu-latest'", runs_on, where)
                if "pull_request" in on:
                    job_if = next((l for l in block.splitlines() if l.strip().startswith("if:")), "")
                    self.assertTrue(FORK_GUARD in runs_on or FORK_GUARD in job_if, f"{where}: a fork's pull request")
                if "pull_request_target" in on:
                    # main's code only: every checkout is main's.
                    self.assertEqual(block.count("uses: actions/checkout"), block.count("ref: main"), where)
        self.assertGreaterEqual(routed, 3)

    def test_the_runners_refuse_forks_themselves(self):
        script = (ROOT / "tools/ci/checks-runners.sh").read_text()
        self.assertIn("ACTIONS_RUNNER_HOOK_JOB_STARTED=$shared/job-started.sh", script)
        self.assertIn('.pull_request.head.repo.full_name // "unknown"', script)
        self.assertIn('if [ "$head" != "${GITHUB_REPOSITORY:-}" ]', script)


if __name__ == "__main__":
    unittest.main()
