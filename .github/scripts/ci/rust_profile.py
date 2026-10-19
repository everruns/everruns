#!/usr/bin/env python3
"""Compare CI builds without retaining experiment caches or changing release artifacts."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import time


def measure(label, command, report, **kwargs):
    start = time.monotonic()
    subprocess.run(command, check=True, **kwargs)
    report[label] = round(time.monotonic() - start, 3)
    print(f'{label}: {report[label]}s', flush=True)


def main():
    mode = sys.argv[1]
    report = dict(revision=os.environ['GITHUB_SHA'], mode=mode)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1']))
    output = Path(os.environ['RUNNER_TEMP']) / f'{mode}-measurements.json'
    try:
        if mode == 'profile':
            report['opt_level'] = os.environ['CARGO_PROFILE_RELEASE_CI_OPT_LEVEL']
            # An empty target is the cold baseline; warm keeps only dependencies,
            # matching production's dependency-only rust-cache policy.
            target = Path(os.environ['CARGO_TARGET_DIR'])
            if target.exists():
                raise RuntimeError('Cold profile trial requires an empty target directory')
            command = ['cargo', 'build', '--locked', '--profile', 'release-ci',
                       '-p', 'everruns-server', '--bin', 'everruns-server',
                       '-p', 'everruns-worker', '--bin', 'everruns-worker',
                       '-p', 'everruns-cli', '--bin', 'everruns']
            measure('cold_compile_seconds', command, report)
            packages = [p['name'] for p in metadata['packages'] if p['id'] in metadata['workspace_members']]
            clean = ['cargo', 'clean', '--profile', 'release-ci']
            for package in packages:
                clean += ['-p', package]
            measure('workspace_clean_seconds', clean, report)
            measure('warm_compile_seconds', command, report)
            binaries = [target/'release-ci'/name for name in ('everruns-server', 'everruns-worker', 'everruns')]
            for binary in binaries:
                # The worker is environment-driven: --help starts it. Its real
                # server/gRPC behavior is verified by the downstream CI suites.
                if binary.name != 'everruns-worker':
                    measure(binary.name + '_help_seconds', [str(binary), '--help'], report)
                sections = subprocess.check_output(['readelf', '-S', str(binary)], text=True)
                if '.debug_line' not in sections or '.symtab' not in sections:
                    raise RuntimeError(f'Missing actionable backtrace information: {binary.name}')
            start = time.monotonic()
            archive = Path(os.environ['RUNNER_TEMP'])/'ci-binaries.tar.gz'
            with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
                for binary in binaries:
                    tar.add(binary, arcname=binary.name)
            report.update(archive_seconds=round(time.monotonic()-start, 3),
                          archive_bytes=archive.stat().st_size,
                          binary_bytes=sum(binary.stat().st_size for binary in binaries))
        elif mode == 'concurrency':
            sys.path.insert(0, str(Path(__file__).parent))
            import libtest
            package = libtest.server_package()
            start = time.monotonic()
            executables = libtest.compile_tests(['cargo', 'test', '--locked', '-p', 'everruns-server',
                '--lib', '--no-run', '--message-format=json'], package)
            report['compile_seconds'] = round(time.monotonic()-start, 3)
            report['trials'] = []
            # Reverse the second pass to expose runner/DB warmup and order effects.
            for threads in (2, 4, 8, 8, 4, 2):
                start = time.monotonic()
                libtest.execute_tests(package['manifest_path'], executables,
                    {key: [f'--test-threads={threads}'] for key in executables})
                report['trials'].append(dict(threads=threads, seconds=round(time.monotonic()-start, 3)))
        else:
            raise ValueError(mode)
    finally:
        output.write_text(json.dumps(report, indent=2)+'\n')
        print(output.read_text(), flush=True)


if __name__ == '__main__':
    main()
