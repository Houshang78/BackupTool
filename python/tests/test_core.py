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

    @unittest.skip(
        "Empty/standalone directory entries were dropped from the manifest in the "
        "1.5.x line so the Python and Rust tools stay manifest-compatible (Rust's "
        "Kind enum is File|Symlink only). Both tools record files and symlinks, not "
        "bare directories. Re-enabling empty-dir backup would require adding a Dir "
        "kind to both implementations.")
    def test_empty_dir_roundtrip(self):
        os.makedirs(os.path.join(self.src, "empty"))
        core.backup([self.src], self.dst, setname="t", log=_silent)
        out = os.path.join(self.tmp, "out")
        core.restore(self.dst, setname="t", target=out, log=_silent)
        restored_rel = self.src.lstrip("/") + "/empty"
        self.assertTrue(os.path.isdir(os.path.join(out, restored_rel)))

    def test_dir_not_counted_as_file(self):
        # Adding/removing files must not let directories inflate file stats.
        res = core.backup([self.src], self.dst, setname="t", log=_silent)
        self.assertEqual(res["copied"], 2)  # a.txt + sub/b.txt only, no dirs

    def test_overlap_detection(self):
        ov = core.analyze_overlaps([self.src, os.path.join(self.src, "sub")])
        covered = {o["path"] for o in ov}
        self.assertIn(os.path.join(self.src, "sub"), covered)

    def test_overlap_none_for_independent(self):
        self.assertEqual(core.analyze_overlaps([self.src, self.dst]), [])

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

    def test_suggested_dest(self):
        d = core.suggested_dest()
        self.assertIn("backuptool-", d)
        self.assertTrue(os.path.isabs(d))

    def test_to_rel_relative(self):
        self.assertEqual(core._to_rel("/Users/x/file"), "Users/x/file")
        self.assertEqual(core._to_rel("\\\\?\\C:\\Users\\x"), "C\\Users\\x")
        self.assertEqual(core._to_rel("D:\\data\\f"), "D\\data\\f")
        self.assertEqual(core._to_rel("C:/Users/x"), "C/Users/x")

    def test_running_blockers_list(self):
        from backuptool import procs
        self.assertIsInstance(procs.running_blockers(), list)

    def test_auto_sources(self):
        a = core.auto_sources()
        self.assertIsInstance(a, list)
        self.assertTrue(a)  # home exists on the dev box
        self.assertEqual(core.scope_sources("auto", []), a)

    def test_volume_root_detection(self):
        for p in ("/", "C:\\", "C:", "\\\\?\\C:\\"):
            self.assertTrue(core._is_volume_root(p), p)
        for p in ("/home/user", "C:\\Users", "/Volumes/Stick"):
            self.assertFalse(core._is_volume_root(p), p)

    def test_disk_excludes_windows_keeps_userdata(self):
        from unittest import mock
        with mock.patch("sys.platform", "win32"):
            ex = core.disk_excludes()
        self.assertTrue(any("Windows" in p for p in ex))
        self.assertTrue(any("Program Files" in p for p in ex))
        # user data must NOT be excluded
        self.assertFalse(any("Users" in p for p in ex))

    def test_scope_sources(self):
        self.assertRaises(ValueError, core.scope_sources, "config", [])
        self.assertEqual(core.scope_sources("config", ["/x"]), ["/x"])
        self.assertRaises(ValueError, core.scope_sources, "bogus", [])

    def test_verify_detects_corruption(self):
        core.backup([self.src], self.dst, setname="v", use_checksum=True, log=_silent)
        set_root = os.path.join(self.dst, "v")
        good = core.verify_set(set_root, log=_silent)
        self.assertEqual(good["errors"], 0)
        # corrupt one copied file at the destination
        with open(os.path.join(set_root, self.src.lstrip("/"), "a.txt"), "w") as f:
            f.write("tampered")
        bad = core.verify_set(set_root, log=_silent)
        self.assertEqual(bad["errors"], 1)

    def test_evacuate_moves_and_verifies(self):
        dest = os.path.join(self.tmp, "evac")
        report = core.evacuate(
            sources=[self.src], dest=dest, setname="e", scope="config",
            delete_source=True, allow_same_device=True, log=_silent)
        self.assertEqual(report["scanned"], 2)
        self.assertEqual(report["verified"], 2)
        self.assertEqual(report["verify_errors"], 0)
        self.assertEqual(report["deleted"], 2)
        # sources gone, verified copies remain
        self.assertFalse(os.path.exists(os.path.join(self.src, "a.txt")))
        v = core.verify_set(os.path.join(dest, "e"), log=_silent)
        self.assertEqual(v["errors"], 0)

    def test_evacuate_dry_run_keeps_sources(self):
        dest = os.path.join(self.tmp, "evacdry")
        report = core.evacuate(
            sources=[self.src], dest=dest, setname="e", scope="config",
            delete_source=True, allow_same_device=True, dry_run=True, log=_silent)
        self.assertEqual(report["deleted"], 0)
        self.assertTrue(os.path.exists(os.path.join(self.src, "a.txt")))

    def test_evacuate_refuses_same_device(self):
        dest = os.path.join(self.tmp, "evac2")
        self.assertRaises(
            RuntimeError, core.evacuate,
            sources=[self.src], dest=dest, setname="e", scope="config",
            delete_source=True, allow_same_device=False, log=_silent)

    def test_evacuate_secure_wipe(self):
        dest = os.path.join(self.tmp, "evacwipe")
        report = core.evacuate(
            sources=[self.src], dest=dest, setname="w", scope="config",
            delete_source=True, secure_wipe=True, wipe_passes=2,
            allow_same_device=True, log=_silent)
        self.assertEqual(report["deleted"], 2)
        self.assertEqual(report["wiped"], 2)
        self.assertFalse(os.path.exists(os.path.join(self.src, "a.txt")))
        v = core.verify_set(os.path.join(dest, "w"), log=_silent)
        self.assertEqual(v["errors"], 0)

    def test_secure_overwrite_truncates(self):
        p = os.path.join(self.src, "a.txt")
        core.secure_overwrite(p, passes=2)
        self.assertEqual(os.path.getsize(p), 0)


if __name__ == "__main__":
    unittest.main()
