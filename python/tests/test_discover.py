# SPDX-License-Identifier: GPL-3.0-or-later
"""Unit tests for the discovery helpers."""
import os
import unittest

from backuptool import discover


class TestDiscover(unittest.TestCase):
    def test_sources_shape(self):
        src = discover.user_data_sources()
        self.assertIsInstance(src, list)
        for c in src:
            self.assertEqual(set(c), {"label", "path", "kind"})
            self.assertIn(c["kind"], ("user", "service", "data", "config"))
            self.assertTrue(os.path.isdir(c["path"]))  # _dedup keeps only real dirs

    def test_destinations_shape(self):
        for c in discover.detect_destinations():
            self.assertEqual(set(c), {"label", "path", "kind"})
            self.assertIn(c["kind"], ("usb", "external", "network"))

    def test_default_destination_is_str(self):
        self.assertIsInstance(discover.default_destination(), str)

    @unittest.skipUnless(os.path.isdir("/etc"), "Linux/Unix only")
    def test_etc_is_a_config_source(self):
        src = discover.user_data_sources()
        self.assertTrue(any(c["path"] == "/etc" and c["kind"] == "config" for c in src))


if __name__ == "__main__":
    unittest.main()
