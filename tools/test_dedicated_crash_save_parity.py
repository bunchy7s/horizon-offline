#!/usr/bin/env python3
"""Expose dedicated player-save and crash-save checks to the parity runner."""
try:
    from .test_dedicated_crash_save import main
except ImportError:
    from test_dedicated_crash_save import main


if __name__ == '__main__':
    raise SystemExit(main())
