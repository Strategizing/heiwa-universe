"""Behavioral checks for local verification evidence, using real subprocesses."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_ci_local import Check, check_inventory, run_checks, stop_process  # noqa: E402


class VerificationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Verification Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "core.hooksPath", os.devnull)
        (self.root / ".gitignore").write_text("private/\n")
        (self.root / "source.txt").write_text("original\n")
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True).strip()

    def check(self, name="fixture", code="print('evidence')", **kwargs):
        return Check(name, (sys.executable, "-c", code), **kwargs)

    def run_gate(self, checks):
        code, path = run_checks(self.root, checks, profile="test")
        return code, json.loads(path.read_text()), path

    def test_pass_binds_source_and_preserves_log(self):
        code, receipt, path = self.run_gate([self.check()])
        self.assertEqual(code, 0)
        self.assertEqual(receipt["status"], "passed")
        self.assertEqual(receipt["source_start"], receipt["source_end"])
        self.assertEqual(receipt["source_start"]["head"], self.git("rev-parse", "HEAD"))
        self.assertFalse(receipt["source_start"]["dirty"])
        self.assertEqual((path.parent / receipt["checks"][0]["log"]).read_text(), "evidence\n")
        if os.name == "posix":
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(path.parent.stat().st_mode & 0o777, 0o700)

    def test_failure_retains_exit_code_and_continues(self):
        code, receipt, _ = self.run_gate([
            self.check("broken", "print('why'); raise SystemExit(17)"),
            self.check("later"),
        ])
        self.assertEqual(code, 1)
        self.assertEqual(receipt["status"], "failed")
        self.assertEqual([c["exit_code"] for c in receipt["checks"]], [17, 0])

    def test_missing_command_fails(self):
        code, receipt, _ = self.run_gate([Check("missing", (str(self.root / "missing"),))])
        self.assertEqual(code, 1)
        self.assertEqual(receipt["checks"][0]["status"], "failed")

    def test_missing_required_gate_fails(self):
        code, receipt, _ = self.run_gate([Check("L0", ("bash", "missing_acceptance.sh"))])
        self.assertEqual(code, 1)
        self.assertEqual(receipt["checks"][0]["status"], "failed")

    def test_inventory_requires_missing_completed_acceptance_gates(self):
        inventory = check_inventory(self.root, full=False, python=sys.executable)
        acceptance = [check for check in inventory if check.name.endswith("acceptance")]
        code, receipt, _ = self.run_gate(acceptance)
        self.assertEqual(code, 1)
        self.assertEqual([row["status"] for row in receipt["checks"]],
                         ["failed", "failed", "failed", "deferred"])

    def test_full_profile_executes_native_certification(self):
        for full in (False, True):
            with self.subTest(full=full):
                inventory = check_inventory(self.root, full=full, python=sys.executable)
                native = next(check for check in inventory if check.name == "native desktop certification")
                self.assertEqual(native.deferred is None, full)
                if full:
                    self.assertEqual(native.argv, ("bash", "scripts/check_native_desktop.sh"))

    def test_deferred_check_is_explicit_and_does_not_run(self):
        code, receipt, _ = self.run_gate([
            self.check(), Check("A1", (), deferred="acceptance is not implemented"),
        ])
        self.assertEqual(code, 0)
        self.assertEqual(receipt["checks"][1]["status"], "deferred")
        self.assertEqual(receipt["checks"][1]["reason"], "acceptance is not implemented")

    def test_no_executed_checks_cannot_pass(self):
        for checks in ([], [Check("A1", (), deferred="pending")]):
            with self.subTest(checks=checks):
                code, receipt, _ = self.run_gate(checks)
                self.assertEqual(code, 1)
                self.assertEqual(receipt["status"], "failed")

    def test_dirty_start_cannot_certify_even_if_check_cleans_it(self):
        (self.root / "untracked.txt").write_text("pending")
        code, receipt, _ = self.run_gate([
            self.check(code="from pathlib import Path; Path('untracked.txt').unlink()")
        ])
        self.assertEqual(code, 1)
        self.assertTrue(receipt["source_start"]["dirty"])
        self.assertFalse(receipt["source_end"]["dirty"])

    def test_dirty_finish_cannot_certify(self):
        code, receipt, _ = self.run_gate([
            self.check(code="from pathlib import Path; Path('source.txt').write_text('changed')")
        ])
        self.assertEqual(code, 1)
        self.assertTrue(receipt["source_end"]["dirty"])

    def test_changed_commit_cannot_certify(self):
        code, receipt, _ = self.run_gate([
            Check("commit", ("git", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "changed"))
        ])
        self.assertEqual(code, 1)
        self.assertNotEqual(receipt["source_start"]["head"], receipt["source_end"]["head"])

    def test_arguments_are_not_shell_code(self):
        literal = "$(touch injected); `touch injected`; 'quoted'"
        code, receipt, path = self.run_gate([
            Check("literal", (sys.executable, "-c", "import sys; print(sys.argv[1])", literal))
        ])
        self.assertEqual(code, 0)
        self.assertEqual((path.parent / receipt["checks"][0]["log"]).read_text().strip(), literal)
        self.assertFalse((self.root / "injected").exists())

    def test_concurrent_runs_keep_separate_receipts_and_logs(self):
        results = []
        threads = [threading.Thread(target=lambda n=n: results.append(self.run_gate([
            self.check(str(n), f"print({n})")
        ]))) for n in range(2)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=10)
            self.assertFalse(thread.is_alive())
        self.assertEqual(len(results), 2)
        self.assertNotEqual(results[0][2], results[1][2])
        for code, receipt, path in results:
            self.assertEqual(code, 0)
            row = receipt["checks"][0]
            self.assertEqual((path.parent / row["log"]).read_text().strip(), row["name"])

    def test_timeout_cannot_pass(self):
        code, receipt, _ = self.run_gate([
            self.check(code="import time; time.sleep(30)", timeout_seconds=0.1)
        ])
        self.assertEqual(code, 1)
        self.assertEqual(receipt["checks"][0]["status"], "timed_out")

    @unittest.skipUnless(os.name == "posix", "POSIX process groups")
    def test_timeout_stops_descendants(self):
        child = "import time; from pathlib import Path; time.sleep(0.6); Path('orphan').touch()"
        parent = f"import subprocess,sys,time; subprocess.Popen([sys.executable, '-c', {child!r}]); time.sleep(30)"
        code, _, _ = self.run_gate([self.check(code=parent, timeout_seconds=0.2)])
        self.assertEqual(code, 1)
        time.sleep(0.7)
        self.assertFalse((self.root / "orphan").exists())

    @unittest.skipUnless(os.name == "posix", "POSIX process groups")
    def test_completed_parent_cannot_leave_source_mutating_descendants(self):
        for exit_code in (0, 17):
            with self.subTest(exit_code=exit_code):
                child = "import time; from pathlib import Path; time.sleep(0.4); Path('source.txt').write_text('late mutation')"
                parent = f"import subprocess,sys; subprocess.Popen([sys.executable, '-c', {child!r}]); sys.exit({exit_code})"
                code, receipt, _ = self.run_gate([self.check(code=parent)])
                self.assertEqual(code, 0 if exit_code == 0 else 1)
                time.sleep(0.5)
                self.assertEqual((self.root / 'source.txt').read_text(), 'original\n')
                self.assertFalse(receipt['source_end']['dirty'])

    @unittest.skipUnless(os.name == "posix", "POSIX process groups")
    def test_cleanup_permission_race_requires_proven_group_absence(self):
        process = subprocess.Popen([sys.executable, '-c', 'pass'], start_new_session=True)
        process.wait()
        with patch('check_ci_local.os.killpg', side_effect=PermissionError('fixture denial')):
            stop_process(process)

    @unittest.skipUnless(os.name == "posix", "POSIX process groups")
    def test_cleanup_denial_for_live_group_is_not_ignored(self):
        process = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'],
                                   start_new_session=True)
        try:
            with patch('check_ci_local.os.killpg', side_effect=PermissionError('fixture denial')):
                with self.assertRaises(PermissionError):
                    stop_process(process)
            self.assertIsNone(process.poll())
        finally:
            stop_process(process)

    def test_cleanup_failure_is_not_recorded_as_a_passing_check(self):
        with patch('check_ci_local.stop_process', side_effect=PermissionError('fixture denial')):
            code, receipt, _ = self.run_gate([self.check()])
        self.assertEqual(code, 1)
        row = receipt['checks'][0]
        self.assertEqual(row['status'], 'cleanup_failed')
        self.assertEqual(row['exit_code'], 0)
        self.assertIn('PermissionError', row['cleanup_error'])
        self.assertIsInstance(row['pid'], int)

    @unittest.skipUnless(os.name == "posix", "POSIX interruption")
    def test_interrupt_keeps_partial_receipt_and_stops_work(self):
        module_dir = str(Path(__file__).resolve().parents[1])
        ready = self.root / "private" / "ready"
        child = f"from pathlib import Path; import time; Path({str(ready)!r}).touch(); time.sleep(30)"
        program = (
            f"import sys,signal; sys.path.insert(0, {module_dir!r}); "
            "from pathlib import Path; from check_ci_local import Check,run_checks; "
            "signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt())); "
            f"sys.exit(run_checks(Path({str(self.root)!r}), "
            f"[Check('active', (sys.executable, '-c', {child!r})), "
            "Check('later', (sys.executable, '-c', 'print(1)'))], profile='test')[0])"
        )
        process = subprocess.Popen([sys.executable, "-c", program], stdout=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 5
            while not ready.exists() and time.monotonic() < deadline and process.poll() is None:
                time.sleep(0.02)
            self.assertTrue(ready.exists())
            process.terminate()
            self.assertEqual(process.wait(timeout=5), 130)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
        paths = list((self.root / "private" / "verification").glob("*/receipt.json"))
        self.assertEqual(len(paths), 1)
        receipt = json.loads(paths[0].read_text())
        self.assertEqual(receipt["status"], "interrupted")
        self.assertEqual([row["status"] for row in receipt["checks"]], ["interrupted"])


if __name__ == "__main__":
    unittest.main()
