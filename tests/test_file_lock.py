from pathlib import Path
import tempfile
import unittest

from tools import file_lock


class FileLockTests(unittest.TestCase):
    def test_a_held_lock_refuses_another_until_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "slot.lock"
            first = open(path, "w")
            second = open(path, "a")
            try:
                self.assertTrue(file_lock.lock(first, wait=False))
                self.assertFalse(file_lock.lock(second, wait=False))
                first.close()
                self.assertTrue(file_lock.lock(second, wait=False))
            finally:
                first.close()
                second.close()


if __name__ == "__main__":
    unittest.main()
