#!/usr/bin/env python3
"""Package one Cargo iOS test executable using existing development signing."""
import argparse
import datetime
import hashlib
import plistlib
import re
import shutil
import subprocess
from pathlib import Path


def checked(command, stage):
    try:
        return subprocess.run(command, check=True, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE).stdout
    except (OSError, subprocess.SubprocessError):
        raise SystemExit(stage + ": failed; inspect signing prerequisites privately") from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', required=True, type=Path)
    parser.add_argument('--profile', required=True, type=Path)
    parser.add_argument('--identity', required=True)
    parser.add_argument('--device-udid', required=True)
    parser.add_argument('--bundle-id', required=True)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Fa-f0-9]{40}', args.identity):
        parser.error('identity must be an existing certificate SHA-1 fingerprint')
    if not re.fullmatch(r'[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+', args.bundle_id):
        parser.error('invalid bundle identifier')
    if args.output.suffix != '.app' or args.output.exists():
        parser.error('output must be a new .app directory')
    profile = plistlib.loads(checked(
        ['security', 'cms', '-D', '-i', str(args.profile)], 'Read development profile'))
    certificates = {hashlib.sha1(cert).hexdigest().upper()
                    for cert in profile['DeveloperCertificates']}
    if args.identity.upper() not in certificates:
        parser.error('identity is not included in this profile')
    if args.device_udid not in profile.get('ProvisionedDevices', []):
        parser.error('device is not included in this development profile')
    if profile['ExpirationDate'].replace(tzinfo=datetime.timezone.utc) <= datetime.datetime.now(datetime.timezone.utc):
        parser.error('development profile has expired')
    allowed = profile['Entitlements']
    team = allowed['com.apple.developer.team-identifier']
    app_prefix, app_pattern = allowed['application-identifier'].split('.', 1)
    identifier = app_prefix + '.' + args.bundle_id
    if not (app_pattern == args.bundle_id or
            app_pattern.endswith('*') and args.bundle_id.startswith(app_pattern[:-1])):
        parser.error('bundle identifier is not permitted by this profile')
    if not allowed.get('get-task-allow'):
        parser.error('an existing development profile with get-task-allow is required')
    entitlements = args.output.parent / (args.output.name + '.entitlements.plist')
    if entitlements.exists():
        parser.error('adjacent scratch entitlements must also be new')
    # Only this new scratch bundle is written. No key, certificate or profile is created.
    args.output.mkdir()
    shutil.copy2(args.executable, args.output / 'RustiaTests')
    shutil.copy2(args.profile, args.output / 'embedded.mobileprovision')
    info = {
        'CFBundleName': 'Rustia Tests', 'CFBundleDisplayName': 'Rustia Tests',
        'CFBundleExecutable': 'RustiaTests', 'CFBundleIdentifier': args.bundle_id,
        'CFBundlePackageType': 'APPL', 'CFBundleVersion': '1',
        'CFBundleShortVersionString': '0.0.1', 'CFBundleSupportedPlatforms': ['iPhoneOS'],
        'UIDeviceFamily': [1, 2], 'MinimumOSVersion': '13.0',
        'LSRequiresIPhoneOS': True, 'UIRequiredDeviceCapabilities': ['arm64'],
        'UILaunchScreen': {},
    }
    (args.output / 'Info.plist').write_bytes(plistlib.dumps(info))
    with entitlements.open('xb') as output:
        output.write(plistlib.dumps({
            'application-identifier': identifier,
            'com.apple.developer.team-identifier': team, 'get-task-allow': True,
        }))
    checked(['codesign', '--force', '--sign', args.identity, '--entitlements',
                    str(entitlements), '--timestamp=none', str(args.output)], 'Sign scratch app')
    checked(['codesign', '--verify', '--strict', str(args.output)], 'Verify scratch app')
    print('Packaged and verified existing development signing in scratch app.')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError):
        raise SystemExit('iOS test packaging failed; inspect local inputs privately') from None
