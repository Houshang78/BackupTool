# SPDX-License-Identifier: GPL-3.0-or-later
#!/usr/bin/env python3
"""Entry script (for PyInstaller and direct execution).

No arguments launches the GUI, otherwise the command line.
"""
from backuptool.__main__ import main

if __name__ == "__main__":
    main()
