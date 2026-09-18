"""Offline installer checks; run with python3 tests/install.py."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[1] / 'scripts/install.sh'


class InstallerTests(unittest.TestCase):
    def run_install(self, system='Linux', arch='x86_64', downloader='curl', failure='', existing=False, args=(), repository=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools = root / 'tools'
            tools.mkdir()
            destination = root / 'install with spaces'
            destination.mkdir()
            if existing:
                (destination / 'qlm').write_text('previous installation')
            for name in ('mkdir', 'mktemp', 'rm', 'chmod', 'mv'):
                (tools / name).symlink_to(shutil.which(name))
            mocks = {
                'uname': '#!/bin/sh\ncase "$1" in -s) echo "$TEST_SYSTEM";; -m) echo "$TEST_ARCH";; esac\n',
                downloader: '''#!/bin/sh
printf '%s\\n' "$@" > "$TEST_LOG"
[ "$TEST_FAILURE" != download ] || exit 22
while [ "$#" -gt 0 ]; do
    case "$1" in -o|-O) output=$2; shift;; esac
    shift
done
[ "$TEST_FAILURE" != empty ] || { : > "$output"; exit 0; }
if [ "$TEST_FAILURE" = incompatible ]; then
    printf '#!/bin/sh\\nexit 1\\n' > "$output"
else
    printf '#!/bin/sh\\necho "qlm test"\\n' > "$output"
fi
''',
            }
            for name, content in mocks.items():
                path = tools / name
                path.write_text(content)
                path.chmod(0o755)
            env = dict(os.environ, PATH=str(tools), QLM_INSTALL_DIR=str(destination),
                       TEST_SYSTEM=system, TEST_ARCH=arch, TEST_FAILURE=failure,
                       TEST_LOG=str(root / 'download.log'))
            env.pop('QLM_REPOSITORY', None)
            if repository is not None:
                env['QLM_REPOSITORY'] = repository
            result = subprocess.run(['/bin/sh', str(INSTALLER), *args], env=env, text=True, capture_output=True)
            installed = destination / 'qlm'
            content = installed.read_text() if installed.exists() else None
            mode = installed.stat().st_mode & 0o777 if installed.exists() else None
            log = (root / 'download.log').read_text() if (root / 'download.log').exists() else ''
            self.assertEqual(list(destination.glob('.qlm-install.*')), [], result.stderr)
            return result, content, mode, log

    def test_platform_assets_and_downloaders(self):
        for system, arch, asset in [('Linux', 'x86_64', 'linux-x86_64'),
                                    ('Darwin', 'x86_64', 'macos-x86_64'),
                                    ('Darwin', 'arm64', 'macos-aarch64')]:
            for downloader in ('curl', 'wget'):
                with self.subTest(system=system, arch=arch, downloader=downloader):
                    result, content, mode, log = self.run_install(system, arch, downloader, existing=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn('qlm test', content)
                    self.assertEqual(mode, 0o755)
                    self.assertIn('/Quartus-Lite-Project-Manager/releases/latest/download/qlm-' + asset, log)

    def test_specific_release_and_repository(self):
        result, _, _, log = self.run_install(args=('v0.2.0',), repository='example/qlm')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('https://github.com/example/qlm/releases/download/v0.2.0/qlm-linux-x86_64', log)

    def test_extra_arguments_rejected(self):
        result, _, _, log = self.run_install(args=('v0.2.0', 'extra'))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(log, '')

    def test_uninstall(self):
        for kind in ('file', 'missing', 'symlink', 'directory'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                destination = root / 'custom bin'
                destination.mkdir()
                binary = destination / 'qlm'
                unrelated = destination / 'other'
                unrelated.write_text('keep')
                if kind == 'file':
                    binary.write_text('binary')
                elif kind == 'symlink':
                    binary.symlink_to(root / 'missing-target')
                elif kind == 'directory':
                    binary.mkdir()
                env = dict(os.environ, QLM_INSTALL_DIR=str(destination))
                result = subprocess.run(['/bin/sh', str(INSTALLER.with_name('uninstall.sh'))],
                                        env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 1 if kind == 'directory' else 0, result.stderr)
                self.assertEqual(unrelated.read_text(), 'keep')
                self.assertEqual(binary.exists(), kind == 'directory')
                self.assertFalse(binary.is_symlink())

    def test_failures_preserve_existing_install(self):
        for failure in ('download', 'empty', 'incompatible'):
            for downloader in ('curl', 'wget'):
                with self.subTest(failure=failure, downloader=downloader):
                    result, content, _, _ = self.run_install(downloader=downloader, failure=failure, existing=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(content, 'previous installation')

    def test_unsupported_platforms_do_not_download(self):
        for system, arch in [('Linux', 'aarch64'), ('Windows_NT', 'x86_64')]:
            result, content, _, log = self.run_install(system, arch)
            self.assertNotEqual(result.returncode, 0)
            self.assertIsNone(content)
            self.assertEqual(log, '')


if __name__ == '__main__':
    unittest.main()
