"""Run a few serial libtest batches, each with its own native PostgreSQL."""
from concurrent.futures import ThreadPoolExecutor, as_completed
from contextlib import ExitStack
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


def run(executable, cwd, env, processes):
    # Raw-pool fixtures deliberately share an explicitly configured database.
    # Preserve their original serialization on nightly/full and local services.
    if any(name in env for name in ('DATABASE_URL', 'DB_PORT', 'EVERRUNS_TEST_POSTGRES_URL')):
        print('External PostgreSQL configured: retaining one serial test process', flush=True)
        subprocess.run([executable, '--test-threads=1'], cwd=cwd, env=env, check=True)
        return
    listing = subprocess.check_output([executable, '--list', '--format', 'terse'],
                                      cwd=cwd, env=env, text=True)
    names = []
    for line in listing.splitlines():
        if not line.endswith(': test'):
            raise RuntimeError(f'Unexpected libtest listing: {line!r}')
        names.append(line.removesuffix(': test'))
    if not names or len(set(names)) != len(names):
        raise RuntimeError('Expected a nonempty, unique libtest inventory')
    groups = [names[index::processes] for index in range(min(processes, len(names)))]
    print(f'Running {len(names)} listed tests in {len(groups)} serial processes', flush=True)
    with tempfile.TemporaryDirectory(prefix='everruns-test-batches-') as directory, ExitStack() as stack:
        ready = Path(directory) / 'postgres-ready'
        batch_env = dict(env, EVERRUNS_TEST_BATCH_READY=str(ready))
        children = []

        def start(index, names):
            log = Path(directory) / f'batch-{index + 1}.log'
            output = stack.enter_context(log.open('w'))
            process = stack.enter_context(subprocess.Popen(
                [executable, '--exact', *names, '--test-threads=1'],
                cwd=cwd, env=batch_env, stdout=output, stderr=subprocess.STDOUT))
            children.append((index, log, process))
            return process

        first = start(0, groups[0])
        while not ready.exists() and first.poll() is None:
            time.sleep(0.02)
        # Unknown binaries without the fixture handshake remain correct and
        # serial; they must not race a cold PostgreSQL installation either.
        concurrent = ready.exists()
        print('Native PostgreSQL ready; starting remaining batches' if concurrent else
              'No native fixture handshake; retaining serial batches', flush=True)
        for index, group in enumerate(groups[1:], start=1):
            if not concurrent:
                children[-1][2].wait()
            start(index, group)

        failed = False
        with ThreadPoolExecutor(max_workers=len(groups)) as pool:
            futures = {pool.submit(process.wait): (index, log) for index, log, process in children}
            for future in as_completed(futures):
                index, log = futures[future]
                code = future.result()
                print(f'=== Serial test batch {index + 1}/{len(groups)} (exit {code}) ===', flush=True)
                with log.open() as output:
                    shutil.copyfileobj(output, sys.stdout)
                failed |= code != 0
        if failed:
            raise subprocess.CalledProcessError(1, [executable])
