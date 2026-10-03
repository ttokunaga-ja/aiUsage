"""Isolated installer checks: mock downloads, preserve HOME and user profiles."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="aiUsage-installer-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin with ' and $ characters"
        self.download = self.root / "downloads"
        self.download.mkdir()
        self.tools = self.root / "tools"
        self.tools.mkdir()
        curl = self.tools / "curl"
        curl.write_text('''#!/bin/sh
set -eu
while [ "$#" -gt 0 ]; do
  if [ "$1" = -o ]; then out=$2; shift 2; else url=$1; shift; fi
done
cp "$AI_USAGE_FIXTURE/${url##*/}" "$out"
''')
        curl.chmod(0o755)
        uname = self.tools / "uname"
        uname.write_text("#!/bin/sh\nprintf 'Darwin\\n'\n")
        uname.chmod(0o755)
        self.env = dict(os.environ, BIN_DIR=str(self.bin),
                        AI_USAGE_INSTALL_NO_PATH="1", AI_USAGE_FIXTURE=str(self.download),
                        PATH=str(self.tools) + ":" + os.environ["PATH"])
        self.asset("#!/bin/sh\nprintf 'aiUsage 0.1.3\\n'\n")

    def asset(self, content):
        data = content.encode()
        (self.download / "aiUsage-macos").write_bytes(data)
        self.entry = hashlib.sha256(data).hexdigest() + "  aiUsage-macos\n"
        (self.download / "SHA256SUMS").write_text(self.entry)

    def run_installer(self):
        return subprocess.run(["sh", str(INSTALLER)], env=self.env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def existing(self):
        self.bin.mkdir()
        target = self.bin / "aiUsage"
        target.write_text("existing executable")
        return target

    def assert_clean(self):
        self.assertEqual(list(self.bin.glob(".aiUsage-install.*")), [])

    def test_install_and_replace(self):
        target = self.existing()
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(target.read_bytes(), (self.download / "aiUsage-macos").read_bytes())
        self.assertTrue(os.access(target, os.X_OK))
        self.assert_clean()

    def test_bad_manifest_and_hash_preserve_existing(self):
        target = self.existing()
        for manifest in ("", self.entry * 2, "0" * 64 + "  aiUsage-macos\n",
                         self.entry.rstrip() + " extra\n"):
            with self.subTest(manifest=manifest):
                (self.download / "SHA256SUMS").write_text(manifest)
                self.assertNotEqual(self.run_installer().returncode, 0)
                self.assertEqual(target.read_text(), "existing executable")
                self.assert_clean()

    def test_invalid_version_preserves_existing(self):
        target = self.existing()
        for content in ("#!/bin/sh\nexit 1\n", "#!/bin/sh\necho unexpected\n",
                        "#!/bin/sh\nprintf 'aiUsage 0.1.3\\naiUsage 0.1.3\\n'\n"):
            self.asset(content)
            self.assertNotEqual(self.run_installer().returncode, 0)
            self.assertEqual(target.read_text(), "existing executable")
            self.assert_clean()

    def test_symlink_is_preserved(self):
        self.bin.mkdir()
        original = self.root / "original"
        original.write_text("symlink target")
        (self.bin / "aiUsage").symlink_to(original)
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertTrue((self.bin / "aiUsage").is_symlink())
        self.assertEqual(original.read_text(), "symlink target")
        self.assert_clean()

    def test_zsh_profile_append_is_literal_and_idempotent(self):
        profile_dir = self.root / "isolated-zdotdir"
        profile_dir.mkdir()
        profile = profile_dir / ".zshrc"
        original = "# Preserve user configuration\nexport USER_SETTING=unchanged\n"
        profile.write_text(original)
        self.env.update(AI_USAGE_INSTALL_NO_PATH="0", SHELL="/bin/zsh", ZDOTDIR=str(profile_dir))
        for _ in range(2):
            result = self.run_installer()
            self.assertEqual(result.returncode, 0, result.stderr)
        text = profile.read_text()
        self.assertTrue(text.startswith(original))
        self.assertEqual(text.count("# aiUsage installer"), 1)
        command = '. "$AI_USAGE_TEST_PROFILE"; . "$AI_USAGE_TEST_PROFILE"; printf "%s" "$PATH"'
        env = dict(self.env, AI_USAGE_TEST_PROFILE=str(profile))
        actual = subprocess.check_output(["sh", "-c", command], env=env, text=True)
        self.assertEqual(actual, str(self.bin) + ":" + self.env["PATH"])


if __name__ == "__main__":
    unittest.main()
