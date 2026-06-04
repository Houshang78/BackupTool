# SPDX-License-Identifier: GPL-3.0-or-later
"""Entry point: no arguments -> GUI, otherwise the command line."""
import sys


def main():
    # Program invoked with no arguments -> launch the GUI.
    if len(sys.argv) == 1:
        try:
            from . import gui
            gui.main()
            return
        except Exception as e:  # Tkinter/PySide6 missing -> hint + CLI help
            sys.stderr.write(f"GUI unavailable ({e}). Using the command line.\n")
    from . import cli
    cli.main()


if __name__ == "__main__":
    main()
