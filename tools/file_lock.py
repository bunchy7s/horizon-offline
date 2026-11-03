"""An exclusive lock on an open file, held until it is closed: flock where it exists, msvcrt's byte lock on Windows."""

import os
import time


def lock(handle, wait=True):
    """Lock `handle`; with wait=False, False at once when another process holds it."""
    if os.name != "nt":
        import fcntl
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | (0 if wait else fcntl.LOCK_NB))
            return True
        except OSError:
            if wait:
                raise
            return False
    import msvcrt
    handle.flush()
    os.lseek(handle.fileno(), 0, os.SEEK_SET)  # the lock covers the first byte, wherever the file was opened
    while True:
        try:
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            return True
        except OSError:
            if not wait:
                return False
            time.sleep(0.1)
