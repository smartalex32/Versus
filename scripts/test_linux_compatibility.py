import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "compatibility", Path(__file__).with_name("check-linux-compatibility.py"))
compatibility = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compatibility)


class LinuxCompatibilityTests(unittest.TestCase):
    def test_needs_are_checked_without_mistaking_definitions_for_requirements(self):
        info = """Version definition section '.gnu.version_d' contains 1 entry:
  0x00: Rev: 1 Flags: none Index: 2 Cnt: 1 Name: GLIBC_2.35
Version needs section '.gnu.version_r' contains 1 entry:
  0x00: Version: 1 File: libc.so.6 Cnt: 2
  0x10: Name: GLIBC_2.28 Flags: none Version: 3
  0x20: Name: GLIBC_2.2.5 Flags: none Version: 4
"""
        versions = compatibility.required_glibc_versions(info)
        self.assertEqual(versions, {"GLIBC_2.28", "GLIBC_2.2.5"})
        self.assertEqual(compatibility.incompatible_versions(versions), [])

    def test_newer_and_private_abi_requirements_fail(self):
        versions = {"GLIBC_2.9", "GLIBC_2.28", "GLIBC_2.29", "GLIBC_2.35",
                    "GLIBC_PRIVATE", "GLIBC_ABI_DT_RELR"}
        self.assertEqual(compatibility.incompatible_versions(versions),
                         ["GLIBC_2.29", "GLIBC_2.35", "GLIBC_ABI_DT_RELR",
                          "GLIBC_PRIVATE"])

    def test_static_runtime_without_version_needs_is_allowed(self):
        self.assertEqual(compatibility.required_glibc_versions(
            "No version information found in this file."), set())

    def test_missing_inputs_and_non_elf_directories_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(ValueError, "Missing compatibility input"):
                compatibility.check_paths([root / "missing"])
            (root / "README").write_text("not an ELF file")
            with self.assertRaisesRegex(ValueError, "No ELF files"):
                compatibility.check_paths([root])


if __name__ == "__main__":
    unittest.main()
