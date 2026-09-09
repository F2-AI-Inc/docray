from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from verify_release_version import verify_release_version


class VerifyReleaseVersionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.manifest = Path(self.temp_dir.name) / "Cargo.toml"
        self.manifest.write_text(
            '[workspace]\n[workspace.package]\nversion = "0.5.2"\n'
        )

    def test_accepts_matching_release_tag(self) -> None:
        self.assertEqual(
            verify_release_version("v0.5.2", self.manifest),
            "0.5.2",
        )

    def test_rejects_tag_that_does_not_match_workspace(self) -> None:
        with self.assertRaisesRegex(
            ValueError,
            "release tag 'v0.5.1' does not match workspace package version '0.5.2'",
        ):
            verify_release_version("v0.5.1", self.manifest)

    def test_rejects_malformed_release_tag(self) -> None:
        for tag in ("0.5.2", "v0.5", "v0.5.2-rc.1", "release-v0.5.2"):
            with self.subTest(tag=tag), self.assertRaisesRegex(
                ValueError, "release tag must be exactly vX.Y.Z"
            ):
                verify_release_version(tag, self.manifest)

    def test_rejects_manifest_without_workspace_version(self) -> None:
        self.manifest.write_text('[workspace]\nmembers = ["crates/*"]\n')

        with self.assertRaises(KeyError):
            verify_release_version("v0.5.2", self.manifest)


if __name__ == "__main__":
    unittest.main()
