# SPDX-License-Identifier: GPL-3.0-or-later
"""Unit tests for Phase 3 reset/clone helpers."""
import sys
import unittest

from backuptool import reset


class TestReset(unittest.TestCase):
    def test_dd_command_well_formed(self):
        cmd = reset.build_clone_command("dd", "/dev/disk2", "/mnt/img.raw")
        self.assertEqual(cmd[0], "dd")
        self.assertIn("if=/dev/disk2", cmd)
        self.assertIn("of=/mnt/img.raw", cmd)

    def test_unknown_tool_raises(self):
        self.assertRaises(ValueError, reset.build_clone_command, "bogus", "a", "b")

    def test_detection_lists_candidates(self):
        tools = reset.detect_clone_tools()
        self.assertTrue(tools)
        if not sys.platform.startswith("win"):
            self.assertTrue(any(t["name"] == "dd" and t["available"] for t in tools))

    def test_reset_command_platform(self):
        cmd = reset.factory_reset_command()
        if sys.platform.startswith("win"):
            self.assertIsNotNone(cmd)
        else:
            self.assertIsNone(cmd)  # macOS/Linux are manual

    def test_instructions_nonempty(self):
        self.assertTrue(reset.instructions().strip())

    def test_storage_type_root(self):
        k = reset.storage_type("/")
        self.assertIn(k, ("ssd", "hdd", "unknown"))
        if sys.platform in ("darwin",) or sys.platform.startswith("linux"):
            self.assertNotEqual(k, "unknown")

    def test_storage_info_names_partition(self):
        dev, kind = reset.storage_info("/")
        self.assertIn(kind, ("ssd", "hdd", "unknown"))
        if sys.platform == "darwin" or sys.platform.startswith("linux"):
            self.assertTrue(dev)  # partition device identified
            self.assertNotEqual(kind, "unknown")

    def test_advice_ssd_mentions_tool(self):
        a = reset.secure_erase_advice("ssd").lower()
        self.assertTrue("cipher" in a or "blkdiscard" in a or "diskutil" in a)
        self.assertIn("overwrite", reset.secure_erase_advice("hdd").lower())

    def test_freespace_cmd_platform(self):
        cmd = reset.windows_freespace_wipe_command("C:\\")
        if sys.platform.startswith("win"):
            self.assertEqual(cmd[0], "cipher")
        else:
            self.assertIsNone(cmd)


if __name__ == "__main__":
    unittest.main()
