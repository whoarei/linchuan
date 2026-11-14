import base64
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from updates import installers, manifest, verify_release, verify_signature


def encode(data):
    return base64.b64encode(data).decode()


class UpdateReleaseTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.key = Ed25519PrivateKey.generate()
        self.key_id = b"keyid123"
        public = self.key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        self.public = encode(("untrusted comment: test key\n" + encode(b"Ed" + self.key_id + public) + "\n").encode())
        for target, file in installers(self.root, "1.2.3").items():
            file.write_bytes(target.encode())
            data_sig = self.key.sign(hashlib.blake2b(file.read_bytes()).digest())
            trusted = f"timestamp:1\tfile:{file.name}\tversion:1.2.3"
            text = "untrusted comment: test signature\n" + encode(b"ED" + self.key_id + data_sig) + "\ntrusted comment: " + trusted + "\n" + encode(self.key.sign(data_sig + trusted.encode())) + "\n"
            Path(str(file) + ".sig").write_text(encode(text.encode()))
        self.feed = manifest(self.root, "v1.2.3", "更新说明")
        self.write_feed()

    def write_feed(self):
        (self.root / "latest.json").write_text(json.dumps(self.feed))

    def test_three_platform_feed_and_signatures_verify(self):
        verify_release(self.root, "v1.2.3", self.public)

    def test_modified_installer_is_rejected(self):
        next(iter(installers(self.root, "1.2.3").values())).write_bytes(b"tampered")
        with self.assertRaises(InvalidSignature):
            verify_release(self.root, "v1.2.3", self.public)

    def test_unexpected_private_files_cannot_be_published(self):
        (self.root / "password.txt").write_text("test fixture")
        with self.assertRaises(ValueError):
            verify_release(self.root, "v1.2.3", self.public)

    def test_old_signed_package_cannot_be_announced_as_new_version(self):
        file = next(iter(installers(self.root, "1.2.3").values()))
        with self.assertRaises(ValueError):
            verify_signature(file, Path(str(file) + ".sig").read_text(), self.public, "9.9.9")

    def test_tampered_trusted_comment_is_rejected(self):
        file = next(iter(installers(self.root, "1.2.3").values()))
        signature = base64.b64decode(Path(str(file) + ".sig").read_text()).replace(b"version:1.2.3", b"version:9.9.9")
        with self.assertRaises(InvalidSignature):
            verify_signature(file, encode(signature), self.public, "9.9.9")

    def test_external_url_and_missing_platform_are_rejected(self):
        target = next(iter(self.feed["platforms"]))
        self.feed["platforms"][target]["url"] = "https://example.com/update.exe"
        self.write_feed()
        with self.assertRaises(ValueError):
            verify_release(self.root, "v1.2.3", self.public)
        del self.feed["platforms"][target]
        self.write_feed()
        with self.assertRaises(ValueError):
            verify_release(self.root, "v1.2.3", self.public)


if __name__ == "__main__":
    unittest.main()
