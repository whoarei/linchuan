import base64
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

import publish_release
from prepare_deb import prepare
from release_policy import metadata, parse_tag, validate_source_version, verify_apk
from updates import installers, manifest


class VersionPolicyTests(unittest.TestCase):
    def test_stable_rc_and_android_versions_order_correctly(self):
        tags = ['v0.3.0', 'v0.4.0-rc.1', 'v0.4.0-rc.2', 'v0.4.0-rc.998', 'v0.4.0', 'v0.4.1-rc.1', 'v0.4.1', 'v0.5.0-rc.1', 'v1.0.0-rc.1', 'v1.0.0']
        codes = [parse_tag(tag).android_code for tag in tags]
        self.assertEqual(codes, sorted(set(codes)))
        self.assertGreater(codes[0], 3000)  # Published 0.3.0's original Android code.
        self.assertTrue(parse_tag('v0.4.0-rc.1').prerelease)
        self.assertFalse(parse_tag('v0.4.0').prerelease)

    def test_invalid_tags_and_android_overflow_are_rejected(self):
        for tag in ['0.4.0', 'v01.0.0', 'v0.4.0-rc.0', 'v0.4.0-rc.01', 'v0.4.0-beta.1', 'v0.4.0+build', 'v0.4.0\n']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                parse_tag(tag)
        for tag in ['v0.4.0-rc.999', 'v0.100.0', 'v0.4.100', 'v210.0.0']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                _ = parse_tag(tag).android_code

    def test_versions_lockfiles_and_notes_must_match_before_build(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            for folder in ['src-tauri', 'crates/core', 'docs/releases']:
                (root / folder).mkdir(parents=True, exist_ok=True)
            version = '0.4.0-rc.1'
            for name in ['package.json', 'src-tauri/tauri.conf.json']:
                (root / name).write_text(json.dumps({'version': version}))
            for name in ['crates/core/Cargo.toml', 'src-tauri/Cargo.toml']:
                (root / name).write_text(f'[package]\nversion = "{version}"\n')
            lock = {'version': version, 'packages': {'': {'version': version}}}
            (root / 'package-lock.json').write_text(json.dumps(lock))
            (root / 'Cargo.lock').write_text('\n'.join(f'[[package]]\nname="{name}"\nversion="{version}"' for name in ['linchuan', 'linchuan-core']))
            validate_source_version(version, root)
            with self.assertRaisesRegex(ValueError, 'notes'):
                metadata('v' + version, root)
            (root / f'docs/releases/{version}.md').write_text('RC notes')
            self.assertEqual(metadata('v' + version, root)['prerelease'], 'true')
            lock['packages']['']['version'] = '0.3.0'
            (root / 'package-lock.json').write_text(json.dumps(lock))
            with self.assertRaises(ValueError):
                validate_source_version(version, root)

    def test_apk_must_have_exact_app_id_version_and_android_code(self):
        line = "package: name='io.github.whoarei.linchuan' versionCode='400001' versionName='0.4.0-rc.1'"
        with patch('release_policy.subprocess.check_output', return_value=line):
            verify_apk('aapt', Path('fixture.apk'), 'v0.4.0-rc.1')
        for invalid in [line.replace('400001', '400000'), line.replace("versionName='0.4.0-rc.1'", "versionName='0.4.0'"), line.replace('io.github.whoarei.linchuan', 'other.application')]:
            with patch('release_policy.subprocess.check_output', return_value=invalid), self.assertRaises(ValueError):
                verify_apk('aapt', Path('fixture.apk'), 'v0.4.0-rc.1')

    @unittest.skipUnless(shutil.which('dpkg-deb'), 'Debian packaging tools required')
    def test_debian_rc_package_orders_before_stable_and_preserves_payload(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            source = root / 'source'
            (source / 'DEBIAN').mkdir(parents=True)
            (source / 'DEBIAN').chmod(0o755)
            (source / 'DEBIAN/control').write_text('Package: linchuan\nVersion: 0.4.0-rc.1\nArchitecture: amd64\nMaintainer: Test <test@example.com>\nDescription: Packaging test\n')
            (source / 'DEBIAN/control').chmod(0o644)
            (source / 'payload').write_bytes(b'original binary bytes')
            packages = root / 'packages'
            packages.mkdir()
            package = packages / 'linchuan.deb'
            subprocess.run(['dpkg-deb', '--build', '--root-owner-group', str(source), str(package)], check=True, capture_output=True)
            prepare(packages, '0.4.0-rc.1')
            version = subprocess.check_output(['dpkg-deb', '-f', str(package), 'Version'], text=True).strip()
            self.assertEqual(version, '0.4.0~rc.1')
            subprocess.run(['dpkg', '--compare-versions', version, 'lt', '0.4.0'], check=True)
            extracted = root / 'extracted'
            subprocess.run(['dpkg-deb', '-x', str(package), str(extracted)], check=True)
            self.assertEqual((extracted / 'payload').read_bytes(), b'original binary bytes')

    def test_artifact_names_are_normalized_and_duplicates_are_rejected(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            source, output = root / 'artifacts', root / 'release'
            source.mkdir()
            for suffix in ['.exe', '.AppImage', '.deb']:
                (source / ('platform_specific_0.4.0~rc.1' + suffix)).write_bytes(suffix.encode())
            args = ['python3', str(Path(__file__).with_name('collect-release.py')), str(source), str(output), '--version', '0.4.0-rc.1']
            subprocess.run(args, check=True, capture_output=True)
            self.assertEqual({path.name for path in output.iterdir()}, {path.name for path in installers(output, '0.4.0-rc.1').values()})
            (source / 'duplicate.exe').write_bytes(b'duplicate')
            self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)


class PublishPolicyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.key = Ed25519PrivateKey.generate()
        self.key_id = b'test-key'
        public = self.key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        encode = lambda value: base64.b64encode(value).decode()
        self.public = encode(('untrusted comment: fixture\n' + encode(b'Ed' + self.key_id + public) + '\n').encode())
        self.notes = self.root.parent / (self.root.name + '-notes.md')
        self.notes.write_text('Release notes')
        self.addCleanup(lambda: self.notes.unlink(missing_ok=True))

    def files(self, version):
        encode = lambda value: base64.b64encode(value).decode()
        for file in installers(self.root, version).values():
            file.write_bytes(file.name.encode())
            signed = self.key.sign(hashlib.blake2b(file.read_bytes()).digest())
            trusted = f'timestamp:1\tversion:{version}'
            text = 'untrusted comment: fixture\n' + encode(b'ED' + self.key_id + signed) + '\ntrusted comment: ' + trusted + '\n' + encode(self.key.sign(signed + trusted.encode())) + '\n'
            Path(str(file) + '.sig').write_text(encode(text.encode()))
        (self.root / f'Linchuan_{version}_android_arm64.apk').write_bytes(b'APK fixture')
        (self.root / 'latest.json').write_text(json.dumps(manifest(self.root, 'v' + version, 'notes')))
        subprocess.run(['python3', str(Path(__file__).with_name('checksums.py')), str(self.root)], check=True)

    def info(self, tag, draft=True):
        return {'tag_name': tag, 'draft': draft, 'prerelease': parse_tag(tag).prerelease, 'html_url': 'https://github.com/example/project/releases/tag/' + tag, 'assets': [{'name': path.name, 'size': path.stat().st_size, 'digest': 'sha256:' + publish_release.digest(path)} for path in self.root.iterdir()]}

    def run_publish(self, tag, responses):
        with patch('publish_release.public_key', return_value=self.public), patch('publish_release.release_info', side_effect=responses), patch('publish_release.command') as command:
            publish_release.publish(self.root, tag, 'example/project', self.notes)
            return [call.args[0] for call in command.call_args_list]

    def test_release_lookup_finds_drafts_and_published_versions_across_pages(self):
        for draft in [True, False]:
            expected = self.info('v0.4.0', draft)
            pages = [[self.info('v0.3.0', False)], [expected]]
            response = subprocess.CompletedProcess([], 0, '\n'.join(json.dumps(info) for page in pages for info in page), '')
            with self.subTest(draft=draft), patch('publish_release.command', return_value=response) as command:
                self.assertEqual(publish_release.release_info('example/project', 'v0.4.0'), expected)
                args = command.call_args.args[0]
                self.assertIn('--paginate', args)
                self.assertEqual(args[-2:], ['--jq', '.[] | @json'])
                self.assertEqual(args[1], 'repos/example/project/releases?per_page=100')

    def test_missing_tag_returns_none_without_creating_a_release(self):
        response = subprocess.CompletedProcess([], 0, json.dumps(self.info('v0.3.0', False)), '')
        with patch('publish_release.command', return_value=response) as command:
            self.assertIsNone(publish_release.release_info('example/project', 'v0.4.0'))
            self.assertEqual(command.call_count, 1)

    def test_ambiguous_drafts_are_rejected(self):
        response = subprocess.CompletedProcess([], 0, '\n'.join(json.dumps(self.info('v0.4.0')) for _ in range(2)), '')
        with patch('publish_release.command', return_value=response), self.assertRaisesRegex(ValueError, 'Multiple releases'):
            publish_release.release_info('example/project', 'v0.4.0')

    def test_release_lookup_errors_are_not_treated_as_missing_versions(self):
        with patch('publish_release.command', side_effect=RuntimeError('HTTP 403')), self.assertRaisesRegex(RuntimeError, 'HTTP 403'):
            publish_release.release_info('example/project', 'v0.4.0')

    def test_stable_publishes_as_latest_after_all_uploads_verify(self):
        tag = 'v0.4.0'
        self.files(tag[1:])
        commands = self.run_publish(tag, [None, self.info(tag), self.info(tag, False)])
        self.assertIn('--draft', commands[0])
        self.assertIn('--latest=false', commands[0])
        self.assertIn('--prerelease=false', commands[-1])
        self.assertIn('--latest=true', commands[-1])

    def test_rc_stays_prerelease_and_never_becomes_latest(self):
        tag = 'v0.4.0-rc.2'
        self.files(tag[1:])
        commands = self.run_publish(tag, [None, self.info(tag), self.info(tag, False)])
        self.assertIn('--prerelease', commands[0])
        self.assertIn('--prerelease=true', commands[-1])
        self.assertTrue(all('--latest=false' in args for args in commands))

    def test_published_versions_are_never_overwritten(self):
        tag = 'v0.4.0'
        self.files(tag[1:])
        with self.assertRaisesRegex(ValueError, 'already published'):
            self.run_publish(tag, [self.info(tag, False)])

    def test_failed_draft_can_resume_but_uploaded_corruption_blocks_publication(self):
        tag = 'v0.4.0-rc.1'
        self.files(tag[1:])
        staged = self.info(tag)
        bad = copy.deepcopy(staged)
        bad['assets'][0]['digest'] = 'sha256:' + '0' * 64
        with patch('publish_release.public_key', return_value=self.public), patch('publish_release.release_info', side_effect=[staged, bad]), patch('publish_release.command') as command:
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                publish_release.publish(self.root, tag, 'example/project', self.notes)
            self.assertEqual(command.call_count, 1)
            self.assertEqual(command.call_args.args[0][:2], ['release', 'upload'])

    def test_tampered_local_checksums_and_private_files_block_any_upload(self):
        tag = 'v0.4.0'
        self.files(tag[1:])
        (self.root / 'SHA256SUMS').write_text('incorrect')
        with patch('publish_release.command') as command, self.assertRaises(ValueError):
            publish_release.verify_files(self.root, parse_tag(tag))
        command.assert_not_called()
        (self.root / 'private.key').write_bytes(b'private fixture')
        with self.assertRaisesRegex(ValueError, 'exactly four'):
            publish_release.verify_files(self.root, parse_tag(tag))

    def test_missing_remote_digest_verifies_downloaded_bytes(self):
        tag = 'v0.4.0'
        self.files(tag[1:])
        info = self.info(tag)
        name = info['assets'][0]['name']
        del info['assets'][0]['digest']
        def download(args):
            shutil.copy2(self.root / name, Path(args[args.index('--dir') + 1]) / name)
        with patch('publish_release.command', side_effect=download) as command:
            publish_release.verify_assets(info, self.root, 'example/project', tag)
        self.assertEqual(command.call_count, 1)


if __name__ == '__main__':
    unittest.main()
