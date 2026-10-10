import importlib.util
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "linux_runtime_libraries", Path(__file__).with_name("linux-runtime-libraries.py"))
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class LinuxRuntimeLibrariesTests(unittest.TestCase):
    def test_native_copy_uses_needed_soname_not_build_host_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            actual = root / "libexample.so.1.2.3"
            actual.write_bytes(b"ELF payload")
            source = root / "libexample.so.1"
            source.symlink_to(actual.name)
            destination = root / "AppDir/usr/lib/native"

            copied = runtime.copy_native([source], destination)

            staged = destination / "libexample.so.1"
            self.assertEqual(copied, {"libexample.so.1": actual.resolve()})
            self.assertTrue(staged.is_file())
            self.assertFalse(staged.is_symlink())
            self.assertEqual(staged.read_bytes(), b"ELF payload")

    def test_same_destination_from_two_sources_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first"
            second = root / "second"
            first.write_bytes(b"one")
            second.write_bytes(b"two")
            copied = {}
            destination = root / "AppDir/usr/lib/libsame.so.1"

            runtime.copy_resolved(first, destination, copied)
            with self.assertRaisesRegex(ValueError, "Two runtime libraries"):
                runtime.copy_resolved(second, destination, copied)

    def test_copy_replaces_host_targeted_destination_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "libsource.so.1"
            source.write_bytes(b"bundled runtime")
            sentinel = root / "host-library"
            sentinel.write_bytes(b"host runtime")
            destination = root / "AppDir/usr/lib/libsource.so.1"
            destination.parent.mkdir(parents=True)
            destination.symlink_to(sentinel)

            runtime.copy_resolved(source, destination, {})

            self.assertFalse(destination.is_symlink())
            self.assertEqual(destination.read_bytes(), b"bundled runtime")
            self.assertEqual(sentinel.read_bytes(), b"host runtime")


if __name__ == "__main__":
    unittest.main()
