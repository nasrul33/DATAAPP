import unittest

from teratai_engine import __version__


class WorkspaceFoundationTest(unittest.TestCase):
    def test_engine_exposes_version(self) -> None:
        self.assertEqual(__version__, "0.1.0")


if __name__ == "__main__":
    unittest.main()
