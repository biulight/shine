"""Regression checks for release ABI admission; no Linux tools required."""

import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "check_linux_glibc", Path(__file__).with_name("check-linux-glibc.py")
)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def needs(*names, flags="none"):
    return "Version needs section '.gnu.version_r' contains 1 entry:\n" + "\n".join(
        f"  0x0010: Name: {name}  Flags: {flags}  Version: 2" for name in names
    )


class GlibcAdmission(unittest.TestCase):
    def test_highest_glibc_requirement_at_or_below_baseline(self):
        definition = "Version definition section: Name: GLIBC_9.0\n"
        for output, expected in [
            (needs("GLIBC_2.2.5", "GLIBC_2.9", "GLIBC_2.35"), (2, 35, 0)),
            (needs("GLIBC_2.9", "GLIBC_2.34"), (2, 34, 0)),
            (definition + needs("GLIBC_2.35", "GCC_4.2.0"), (2, 35, 0)),
        ]:
            with self.subTest(output=output):
                self.assertEqual(checker.check_versions(output), expected)

    def test_rejects_newer_versions_including_weak_requirements(self):
        for version in ("2.35.1", "2.36", "2.38", "2.39", "2.100", "3.0"):
            output = needs("GLIBC_2.2.5", f"GLIBC_{version}", flags="WEAK")
            with self.subTest(version=version), self.assertRaisesRegex(
                ValueError, "exceeds"
            ):
                checker.check_versions(output)

    def test_rejects_private_or_unrecognized_glibc_requirements(self):
        for name in ("GLIBC_PRIVATE", "GLIBC_ABI_DT_RELR", "GLIBC_bad"):
            with self.subTest(name=name), self.assertRaisesRegex(
                ValueError, "unsupported"
            ):
                checker.check_versions(needs("GLIBC_2.35", name))

    def test_rejects_missing_or_empty_requirements(self):
        for output in ("", "not an ELF", needs(), needs("GCC_3.0")):
            with self.subTest(output=output), self.assertRaisesRegex(
                ValueError, "no glibc"
            ):
                checker.check_versions(output)


if __name__ == "__main__":
    unittest.main()
