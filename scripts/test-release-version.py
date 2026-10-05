import importlib.util
import pathlib
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("release_version", pathlib.Path(__file__).with_name("release-version.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseVersionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Version test")
        self.git("config", "user.email", "test@example.invalid")
        self.manifest = self.root / "Cargo.toml"
        self.manifest.write_text('[package]\nname = "signal-forge"\nversion = "0.1.1"\n\n[dependencies]\n')
        (self.root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "signal-forge"\nversion = "0.1.1"\n\n[[package]]\nname = "dependency"\nversion = "1.0.0"\n')
        self.commit("base")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True, stderr=subprocess.DEVNULL).strip()

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "--allow-empty", "-m", message)

    def test_each_merge_increments_once_and_reruns_are_stable(self):
        self.assertEqual(release.release_version(self.root), "0.1.1")
        self.git("checkout", "-b", "feature")
        for i in range(3):
            self.commit(f"feature {i}")
        self.git("checkout", "main")
        self.git("merge", "--no-ff", "feature", "-m", "merge PR")
        self.assertEqual(release.release_version(self.root), "0.1.2")
        self.assertEqual(release.release_version(self.root), "0.1.2")
        self.commit("squash merge or direct push")
        self.assertEqual(release.release_version(self.root), "0.1.3")

    def test_dependency_edits_do_not_reset_version(self):
        self.commit("merge")
        self.manifest.write_text(self.manifest.read_text() + 'serde = "1"\n')
        self.commit("dependencies")
        self.assertEqual(release.release_version(self.root), "0.1.3")

    def test_new_base_introduced_in_pr_is_used_on_merge(self):
        self.git("checkout", "-b", "version-bump")
        self.manifest.write_text(self.manifest.read_text().replace('"0.1.1"', '"0.2.0"'))
        self.commit("new base")
        self.commit("another feature commit")
        self.git("checkout", "main")
        self.git("merge", "--no-ff", "version-bump", "-m", "merge version PR")
        self.assertEqual(release.release_version(self.root), "0.2.0")
        self.commit("next merge")
        self.assertEqual(release.release_version(self.root), "0.2.1")

    def test_explicit_base_version_change_starts_new_series(self):
        self.commit("merge")
        self.manifest.write_text(self.manifest.read_text().replace('"0.1.1"', '"0.2.0"'))
        self.commit("minor release")
        self.assertEqual(release.release_version(self.root), "0.2.0")
        self.commit("next merge")
        self.assertEqual(release.release_version(self.root), "0.2.1")

    def test_build_version_updates_only_root_package_and_is_idempotent(self):
        self.commit("merge")
        version = release.release_version(self.root)
        release.apply_version(self.root, version)
        release.apply_version(self.root, release.release_version(self.root))
        self.assertEqual(release.package_version(self.manifest.read_text()), "0.1.2")
        packages = release.tomllib.loads((self.root / "Cargo.lock").read_text())["package"]
        self.assertEqual([p["version"] for p in packages], ["0.1.2", "1.0.0"])

    def test_invalid_version_and_missing_lock_entry_leave_files_unchanged(self):
        before = self.manifest.read_text()
        with self.assertRaises(ValueError):
            release.apply_version(self.root, "0.1.2-rc1")
        (self.root / "Cargo.lock").write_text('version = 4\n')
        with self.assertRaises(ValueError):
            release.apply_version(self.root, "0.1.2")
        self.assertEqual(self.manifest.read_text(), before)


if __name__ == "__main__":
    unittest.main()
