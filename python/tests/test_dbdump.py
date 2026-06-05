# SPDX-License-Identifier: GPL-3.0-or-later
"""Unit tests for the database dump adapters."""
import os
import shutil
import tempfile
import unittest

from backuptool import dbdump


def _silent(*_a, **_k):
    pass


class TestDbDump(unittest.TestCase):
    def test_detect_shape(self):
        for d in dbdump.detect_databases():
            self.assertEqual(set(d), {"name", "kind"})
            self.assertIn(d["kind"], ("postgresql", "mysql", "redis", "mongodb"))

    def test_generic_dump_runs(self):
        tmp = tempfile.mkdtemp()
        try:
            spec = {"name": "demo", "shell": 'printf data > "$BACKUPTOOL_DB_OUT/demo.txt"'}
            self.assertTrue(dbdump.dump_database(spec, tmp, log=_silent))
            self.assertTrue(os.path.exists(os.path.join(tmp, "demo.txt")))
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    def test_generic_dump_failure_is_reported(self):
        tmp = tempfile.mkdtemp()
        try:
            spec = {"name": "boom", "shell": "exit 3"}
            self.assertFalse(dbdump.dump_database(spec, tmp, log=_silent))
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
