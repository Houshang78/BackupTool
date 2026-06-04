# SPDX-License-Identifier: GPL-3.0-or-later
"""Unit tests for the core engine (run: python3 -m unittest discover -s tests)."""
import os
import shutil
import tempfile
import unittest

from backuptool import core


def _silent(*_a, **_k):
    pass


class TestCore(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.src = os.path.join(self.tmp, "src")
        self.dst = os.path.join(self.tmp, "dst")
        os.makedirs(os.path.join(self.src, "sub"))
        self._write("a.txt", "hello")
        self._write(os.path.join("sub", "b.txt"), "world")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _write(self, rel, text):
        with open(os.path.join(self.src, rel), "w", encoding="utf-8") as f:
            f.write(text)

    def test_backup_then_restore_roundtrip(self):
        core.backup([self.src], self.dst, setname="t", log=_silent)
        out = os.path.join(self.tmp, "out")
        core.restore(self.dst, setname="t", target=out, log=_silent)
        restored = []
        for _root, _dirs, files in os.walk(out):
            restored.extend(files)
        self.assertIn("a.txt", restored)
        self.assertIn("b.txt", restored)

    def test_incremental_skips_unchanged(self):
        core.backup([self.src], self.dst, setname="t", log=_silent)
        res = core.backup([self.src], self.dst, setname="t", log=_silent)
        self.assertEqual(res["copied"], 0)
        self.assertEqual(res["skipped"], 2)

    def test_change_is_detected(self):
        core.backup([self.src], self.dst, setname="t", log=_silent)
        self._write("a.txt", "changed content")
        res = core.backup([self.src], self.dst, setname="t", log=_silent)
        self.assertEqual(res["copied"], 1)

    def test_needs_copy(self):
        a = {"type": "file", "size": 10, "mtime": 100}
        self.assertTrue(core.needs_copy(a, None, False))
        self.assertFalse(core.needs_copy(a, {"type": "file", "size": 10, "mtime": 100}, False))
        self.assertTrue(core.needs_copy(a, {"type": "file", "size": 11, "mtime": 100}, False))

    def test_list_sets(self):
        core.backup([self.src], self.dst, setname="myhost", log=_silent)
        sets = core.list_sets(self.dst)
        self.assertEqual(len(sets), 1)
        self.assertEqual(sets[0]["set"], "myhost")

    def test_log_written(self):
        res = core.backup([self.src], self.dst, setname="t", log=_silent)
        self.assertTrue(res["log"] and os.path.exists(res["log"]))
        with open(res["log"], encoding="utf-8") as f:
            text = f.read()
        self.assertIn("CHANGED", text)
        self.assertIn("# backuptool", text)

    def test_system_dirs_exist(self):
        for d in core.system_dirs():
            self.assertTrue(os.path.isdir(d))


if __name__ == "__main__":
    unittest.main()
