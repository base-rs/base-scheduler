#!/usr/bin/env python3
"""Exercise profiler cleanup; use PROFILE_TEST_SUDO=1 for the privileged path."""

import os
from pathlib import Path
import select
import subprocess
import sys
import time
import unittest


class SupervisorTests(unittest.TestCase):
    def start(self, workload, timeout=2):
        prefix = ["sudo", "-n"] if os.environ.get("PROFILE_TEST_SUDO") == "1" else []
        invoke = "import runpy,sys; sys.exit(runpy.run_path(sys.argv[1])['supervise'](sys.argv[3:], float(sys.argv[2])))"
        child = subprocess.Popen(
            prefix + [sys.executable, "-c", invoke, str(Path(__file__).with_name("supervise.py")),
                      str(timeout), sys.executable, "-c", workload],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        self.addCleanup(self.retire, child)
        return child

    @staticmethod
    def retire(child):
        child.stdin.close()
        child.wait(timeout=5)
        child.stdout.close()
        child.stderr.close()

    def line(self, child):
        self.assertTrue(select.select([child.stdout], [], [], 5)[0], "missing child output")
        return child.stdout.readline().strip()

    def assert_retired(self, pid):
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            status = Path(f"/proc/{pid}/stat")
            if not status.exists() or status.read_text().split()[2] == "Z":
                return
            time.sleep(0.01)
        self.fail(f"descendant {pid} remains running")

    def test_exit_status(self):
        child = self.start("raise SystemExit(7)")
        self.assertEqual(child.wait(timeout=5), 7)

    def test_caller_closes_pipe(self):
        child = self.start("import os,time; print(os.getpid(), flush=True); time.sleep(60)")
        pid = int(self.line(child))
        child.stdin.close()
        self.assertNotEqual(child.wait(timeout=5), 0)
        self.assert_retired(pid)

    def test_wrapper_exits_before_descendant(self):
        workload = "import subprocess,sys; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); print(p.pid,flush=True)"
        child = self.start(workload)
        pid = int(self.line(child))
        self.assertEqual(child.wait(timeout=5), 0)
        self.assert_retired(pid)

    def test_timeout(self):
        child = self.start("import os,time; print(os.getpid(), flush=True); time.sleep(60)", timeout=0.3)
        pid = int(self.line(child))
        self.assertNotEqual(child.wait(timeout=5), 0)
        self.assert_retired(pid)


if __name__ == "__main__":
    unittest.main()
