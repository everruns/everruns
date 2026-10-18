#!/usr/bin/env python3
"""Execute Cargo's libtest artifacts without a second build invocation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

import test_batches


def server_package():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version', '1'], text=True))
    return next(p for p in metadata['packages'] if p['name'] == 'everruns-server')


def discover(lines, package):
    executables = {}
    for line in lines:
        artifact = json.loads(line)
        if artifact.get('reason') == 'compiler-message':
            print(artifact['message'].get('rendered', ''), end='', file=sys.stderr)
        if (artifact.get('reason') != 'compiler-artifact'
                or artifact.get('package_id') != package['id']
                or not artifact.get('executable') or not artifact['profile']['test']):
            continue
        target = artifact['target']
        executables[(target['name'], target['kind'][0])] = str(Path(artifact['executable']).resolve())
    return executables


def compile_tests(command, package, targets=None, artifacts=None):
    start = time.monotonic()
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True) as process:
        def lines():
            for line in process.stdout:
                if artifacts is not None:
                    artifacts.write(line)
                yield line
        executables = discover(lines(), package)
        if process.wait():
            raise subprocess.CalledProcessError(process.returncode, command)
    print(f'Selected server compilation: {time.monotonic() - start:.2f}s', flush=True)
    if targets is None:
        return executables
    missing = targets.keys() - executables.keys()
    if missing:
        raise RuntimeError(f'Missing selected test executables: {missing}')
    return {key: executables[key] for key in targets}


def execute_tests(manifest, executables, targets, batch_targets=(), processes=2):
    # Cargo supplies package cwd and manifest context to fixture readers.
    manifest = Path(manifest).resolve()
    env = dict(os.environ, CARGO_MANIFEST_DIR=str(manifest.parent), CARGO_MANIFEST_PATH=str(manifest))
    for key, arguments in targets.items():
        start = time.monotonic()
        if key[0] in batch_targets:
            if arguments not in ([], ['--test-threads=1']):
                raise ValueError('Batch execution requires an unfiltered serial target')
            test_batches.run(executables[key], manifest.parent, env, processes)
        else:
            subprocess.run([executables[key], *arguments], cwd=manifest.parent, env=env, check=True)
        print(f'Selected server {key[0]} execution: {time.monotonic() - start:.2f}s', flush=True)


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    compile = commands.add_parser('compile')
    compile.add_argument('artifacts', type=Path)
    compile.add_argument('arguments', nargs=argparse.REMAINDER)
    run = commands.add_parser('run')
    run.add_argument('artifacts', type=Path)
    run.add_argument('--serial', action='append', default=[])
    run.add_argument('--batch', action='append', default=[])
    run.add_argument('--processes', type=int, choices=[2, 4], default=2)
    archive = commands.add_parser('archive-workflow')
    archive.add_argument('artifacts', type=Path)
    archive.add_argument('directory', type=Path)
    execute = commands.add_parser('execute-workflow')
    execute.add_argument('directory', type=Path)
    execute.add_argument('arguments', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command == 'execute-workflow':
        executable = (args.directory / 'workflow-tests').resolve()
        if digest(executable) != (args.directory / 'workflow-tests.sha256').read_text().strip():
            raise RuntimeError('Workflow test executable checksum mismatch')
        executable.chmod(0o755)
        arguments = args.arguments[1:] if args.arguments[:1] == ['--'] else args.arguments
        execute_tests(Path('crates/server/Cargo.toml'), {('workflow_test', 'test'): str(executable)},
                      {('workflow_test', 'test'): arguments})
        return
    package = server_package()
    if args.command == 'compile':
        with args.artifacts.open('w') as output:
            compile_tests(args.arguments[1:] if args.arguments[:1] == ['--'] else args.arguments, package, artifacts=output)
        return
    with args.artifacts.open() as source:
        executables = discover(source, package)
    if not executables:
        raise RuntimeError('No server test executables emitted')
    if args.command == 'archive-workflow':
        if set(executables) != {('workflow_test', 'test')}:
            raise RuntimeError(f'Expected only workflow_test, got {list(executables)}')
        args.directory.mkdir(parents=True, exist_ok=True)
        executable = args.directory / 'workflow-tests'
        shutil.copyfile(executables[('workflow_test', 'test')], executable)
        (args.directory / 'workflow-tests.sha256').write_text(digest(executable) + '\n')
    else:
        missing = set(args.serial + args.batch) - {name for name, _ in executables}
        if missing:
            raise RuntimeError(f'Missing selected test targets: {missing}')
        targets = {key: ['--test-threads=1'] if key[0] in args.serial else [] for key in executables}
        execute_tests(package['manifest_path'], executables, targets, args.batch, args.processes)


if __name__ == '__main__':
    main()
