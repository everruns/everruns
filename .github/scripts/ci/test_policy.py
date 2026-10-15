#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import re
import unittest
import os
import subprocess
import tempfile
import json
import hashlib
import shutil
import tarfile

import yaml

ROOT = Path(__file__).resolve().parents[3]


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name+'.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


plan, focused, tracker, release = map(load, ['plan','focused','track_failures','wait_full'])


def evaluate(expression, values):
    expression = expression.removeprefix('${{').removesuffix('}}').strip()
    expression = re.sub(r'\b(?:github|needs|steps|matrix)(?:\.[a-zA-Z_][a-zA-Z_0-9-]*)+',
                        lambda m: repr(values.get(m.group(), '')), expression)
    return bool(eval(expression.replace('&&',' and ').replace('||',' or '), {'__builtins__':{}}, {'always':lambda:True}))


class Policy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = yaml.safe_load((ROOT/'.github/workflows/ci.yml').read_text())
        cls.jobs = cls.workflow['jobs']

    def test_large_ci_rust_caches_are_read_only_outside_main(self):
        producers = 0
        for job in self.jobs.values():
            for step in job.get('steps', []):
                if not step.get('uses', '').startswith('Swatinem/rust-cache@'):
                    continue
                condition = step.get('with', {}).get('save-if', True)
                if condition is False:
                    continue
                producers += 1
                self.assertIsInstance(condition, str)
                for event, ref in [('pull_request', 'refs/pull/7/merge'),
                                   ('workflow_dispatch', 'refs/heads/experiment')]:
                    self.assertFalse(evaluate(condition, {'github.event_name': event, 'github.ref': ref,
                        'matrix.shard': 'domain', 'needs.changes.outputs.rust': 'true'}))
        self.assertGreater(producers, 5)

    def test_cleanup_preserves_active_pr_and_all_branch_caches(self):
        cleanup = load('cache_cleanup')
        def cache(identity, ref):
            return dict(id=identity, ref=ref, size_in_bytes=100)
        caches = [cache(1, 'refs/heads/main'), cache(2, 'refs/heads/topic'),
                  cache(3, 'refs/pull/7/merge'), cache(4, 'refs/pull/8/merge'),
                  cache(5, 'refs/pull/7/head'), cache(6, 'refs/pull/9/merge')]
        deleted = []
        def request(endpoint, method='GET'):
            if method == 'DELETE':
                deleted.append(int(endpoint.rsplit('/', 1)[1])); return
            if '/actions/caches?' in endpoint:
                return [{'actions_caches': caches[:3]}, {'actions_caches': caches[3:]}]
            number = int(endpoint.rsplit('/', 1)[1])
            return [dict(number=number, state='closed' if number != 8 else 'open',
                         base={'repo': {'full_name': 'owner/repo' if number != 9 else 'other/repo'}})]
        self.assertEqual(cleanup.cleanup('owner/repo', request=request), (1, 100))
        self.assertEqual(deleted, [3])
        deleted.clear()
        self.assertEqual(cleanup.cleanup('owner/repo', number=8, request=request), (0, 0))
        self.assertEqual(deleted, [])

    def test_cleanup_rechecks_reopened_pr_before_each_delete(self):
        cleanup = load('cache_cleanup')
        states = iter(['closed', 'open'])
        deleted = []
        def request(endpoint, method='GET'):
            if method == 'DELETE':
                deleted.append(endpoint); return
            if '/actions/caches?' in endpoint:
                return [{'actions_caches': [dict(id=i, ref='refs/pull/7/merge', size_in_bytes=10) for i in (1, 2)]}]
            return [dict(number=7, state=next(states), base={'repo': {'full_name': 'owner/repo'}})]
        self.assertEqual(cleanup.cleanup('owner/repo', request=request), (1, 10))
        self.assertEqual(len(deleted), 1)

    def test_privileged_cleanup_uses_only_trusted_main(self):
        workflow = yaml.safe_load((ROOT/'.github/workflows/ci-cache-cleanup.yml').read_text())
        self.assertEqual(workflow[True]['pull_request_target']['types'], ['closed'])
        self.assertIn('github.event.pull_request.number', workflow['concurrency']['group'])
        job = workflow['jobs']['cleanup']
        self.assertEqual(job['permissions'], {'contents': 'read', 'actions': 'write', 'pull-requests': 'read'})
        checkout = job['steps'][0]
        self.assertRegex(checkout['uses'], r'actions/checkout@[0-9a-f]{40}$')
        self.assertEqual(checkout['with'], {'ref': 'main', 'persist-credentials': False})
        self.assertNotIn('secrets.', json.dumps(workflow))

    def test_baseline_never_advances_on_failure_cancel_or_pr(self):
        runs = [dict(id=n,event=e,conclusion=c,head_branch='main',head_sha=sha,display_title='CI batch '+sha)
                for n,e,c,sha in [(1,'push','success','good'),(2,'push','failure','failed'),
                                   (3,'push','cancelled','cancelled'),(4,'pull_request','success','pr'),
                                   (5,'push',None,'running')]]
        self.assertEqual(plan.successful_base(runs,'now',lambda a,b:True),'good')
        self.assertEqual(plan.successful_base(runs,'now',lambda a,b:False),'')

    def test_release_identity_and_plan_cannot_disagree_on_case(self):
        self.assertEqual(plan.mode('push',False,'CHORE(RELEASE): PREPARE V1.0.0'),'full')
        self.assertEqual(plan.mode('pull_request',True,'chore(release): prepare v1.0.0'),'pr')
        self.assertEqual(plan.mode('workflow_dispatch',True,'ordinary commit'),'full')

    def test_empty_bootstrap_and_diverged_history(self):
        self.assertEqual(plan.successful_base([], 'new', lambda a,b:True),'')
        run=dict(id=1,event='push',conclusion='success',head_branch='main',head_sha='new',display_title='CI batch new')
        self.assertEqual(plan.successful_base([run],'new',lambda a,b:True),'')

    def test_reverse_dependency_and_markdown_selection(self):
        def package(name,deps=()):
            return dict(name=name,manifest_path=f'/repo/crates/{name}/Cargo.toml',dependencies=[{'name':n} for n in deps])
        packages=[package('contract'),package('driver',['contract']),package('server',['driver']),package('other')]
        self.assertEqual(focused.affected(packages,['crates/contract/src/lib.rs'],Path('/repo')),{'contract','driver','server'})
        self.assertEqual(focused.affected(packages,['crates/contract/README.md'],Path('/repo')),set())
        self.assertEqual(focused.affected(packages,['Cargo.lock'],Path('/repo')),{'contract','driver','server','other'})
        self.assertEqual(focused.affected(packages,['crates/driver/src/deleted.rs'],Path('/repo')),{'driver','server'})

    def test_security_and_migration_changes_add_contracts(self):
        self.assertIn('auth_integration_test::',focused.risk_filters(['crates/server/src/auth/api_keys.rs']))
        self.assertIn('org_isolation_test::',focused.risk_filters(['crates/server/src/storage/runtime_identity/mod.rs']))
        self.assertIn('auth_integration_test::',focused.risk_filters(['.github/scripts/ci/focused.py']))
        self.assertIn('migration_history_test::',focused.risk_filters(['crates/server/migrations/20261014000000_test.sql']))
        self.assertEqual(focused.risk_filters(['crates/core/src/engine.rs']),[])
        for path in ['crates/server/src/api/monitoring/mod.rs', 'crates/server/src/openapi/mod.rs']:
            filters = focused.risk_filters([path])
            for contract in ['openapi_coverage_test::', 'openapi_descriptions_test::', 'pat_scope_coverage_test::']:
                self.assertIn(contract, filters)
            self.assertNotIn('auth_integration_test::', filters)
        self.assertIn('service_mcp_oauth_lifecycle_test::', focused.risk_filters(['crates/server/tests/test_harness.rs']))

    def test_database_backend_changes_run_real_router_fixtures(self):
        targets=['background_sweep_wiring_test','pat_scope_sweep_test']
        for path in ['crates/server/src/storage/test_database.rs',
                     'crates/server/src/storage/embedded_postgres/mod.rs',
                     'crates/server/tests/test_harness/database.rs']:
            self.assertEqual(focused.real_server_targets([path]),targets)
        server=dict(id='server',name='everruns-server',manifest_path='/repo/crates/server/Cargo.toml',dependencies=[],targets=[{'kind':['lib']}])
        selection=focused.select({'packages':[server],'workspace_members':['server']},
                                 ['crates/server/src/storage/test_database.rs'],Path('/repo'))['include'][0]
        self.assertEqual(selection['risk_targets'],targets)
        command=next(focused.commands(selection))
        for target in targets:self.assertIn(target,command)
        self.assertEqual(focused.real_server_targets(['crates/server/tests/pat_scope_sweep_test.rs']),['pat_scope_sweep_test'])
        self.assertEqual(focused.real_server_targets(['crates/server/src/domains/models/service.rs']),[])

    def test_selected_server_artifacts_run_with_filters_and_cargo_context(self):
        selection = dict(packages=['everruns-server'], risk_targets=['router'], risk_filters=['picked::'])
        commands = list(focused.commands(selection))
        self.assertEqual(len(commands), 1)
        self.assertIn('--no-run', commands[0])
        self.assertIn('domain', commands[0])
        # A real tiny Cargo workspace proves compilation, artifact discovery,
        # package context, domain filtering, and propagation of libtest failures.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'src').mkdir(); (root/'tests').mkdir()
            (root/'Cargo.toml').write_text('[package]\nname="everruns-server"\nversion="0.1.0"\nedition="2024"\n[workspace]\n')
            fixture = '''
#[test]
fn cargo_context() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    assert_eq!(std::env::current_dir().unwrap(), std::path::Path::new(&manifest));
    assert_eq!(std::env::var("CARGO_MANIFEST_PATH").unwrap(), format!("{manifest}/Cargo.toml"));
}
'''
            (root/'src/lib.rs').write_text(fixture)
            for target in ['contracts', 'router']:
                (root/f'tests/{target}.rs').write_text(fixture)
            (root/'tests/domain.rs').write_text('''
mod picked {
    #[test]
    fn selected() { assert!(std::env::var("FAIL_SELECTED").is_err(), "selected failure"); }
}
#[test]
fn unselected() { panic!("filter was lost"); }
''')
            env = dict(os.environ, TEST_SELECTION=json.dumps(selection), CARGO_TARGET_DIR=str(root/'target'))
            env.pop('CARGO_MANIFEST_DIR', None); env.pop('CARGO_MANIFEST_PATH', None)
            env.pop('FAIL_SELECTED', None)
            subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=root, env=env, check=True, capture_output=True)
            command = ['python3', str(ROOT/'.github/scripts/ci/focused.py'), '--run']
            result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
            self.assertEqual(result.stdout.count('1 passed; 0 failed'), 4)
            env['FAIL_SELECTED'] = '1'
            failed = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn('selected failure', failed.stdout)

    def test_main_artifacts_preserve_threads_and_survive_transfer(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root/'crates/server'
            (package/'src').mkdir(parents=True); (package/'tests').mkdir()
            (package/'Cargo.toml').write_text('[package]\nname="everruns-server"\nversion="0.1.0"\nedition="2024"\n[workspace]\n')
            context = '''
fn context() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    assert_eq!(std::env::current_dir().unwrap(), std::path::Path::new(&manifest));
    assert_eq!(std::env::var("CARGO_MANIFEST_PATH").unwrap(), format!("{manifest}/Cargo.toml"));
}
'''
            (package/'src/lib.rs').write_text(context+'#[test] fn unit() { context(); }')
            (package/'tests/contracts.rs').write_text(context+'#[test] fn contract() { context(); }')
            (package/'tests/server_integration.rs').write_text(context+'''
#[test] fn serial() {
    context();
    assert!(std::env::args().any(|arg| arg == "--test-threads=1"));
}
''')
            (package/'tests/workflow_test.rs').write_text(context+'''
#[test] fn offline() { context(); }
#[test] fn paid() { panic!("paid case was not excluded"); }
''')
            env = dict(os.environ, CARGO_TARGET_DIR=str(root/'target'))
            script = str(ROOT/'.github/scripts/ci/libtest.py')
            subprocess.run(['cargo','generate-lockfile','--offline'],cwd=package,env=env,check=True,capture_output=True)
            build = ['cargo','test','--locked','-p','everruns-server','--no-run','--message-format=json']
            artifacts = root/'artifacts.jsonl'
            subprocess.run(['python3',script,'compile',str(artifacts),'--',*build,
                            '--lib','--test','contracts','--test','server_integration'],
                           cwd=package,env=env,check=True,capture_output=True)
            result = subprocess.run(['python3',script,'run',str(artifacts),'--serial','server_integration'],
                                    cwd=package,env=env,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
            self.assertEqual(result.stdout.count('1 passed; 0 failed'),3)
            with artifacts.open('w') as output:
                subprocess.run([*build,'--test','workflow_test'],cwd=package,env=env,stdout=output,stderr=subprocess.PIPE,check=True)
            archive = root/'transferred'
            subprocess.run(['python3',script,'archive-workflow',str(artifacts),str(archive)],
                           cwd=package,env=env,check=True,capture_output=True)
            shutil.rmtree(root/'target')
            (archive/'workflow-tests').chmod(0o644)  # download-artifact drops executable mode.
            command = ['python3',script,'execute-workflow',str(archive),'--','--test-threads=1','--skip','paid']
            result = subprocess.run(command,cwd=root,env=env,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
            self.assertIn('1 passed; 0 failed',result.stdout)
            unfiltered = subprocess.run(command[:-2],cwd=root,env=env,capture_output=True,text=True)
            self.assertNotEqual(unfiltered.returncode,0)
            self.assertIn('paid case was not excluded',unfiltered.stdout)
            with (archive/'workflow-tests').open('ab') as output: output.write(b'changed')
            changed = subprocess.run(command,cwd=root,env=env,capture_output=True,text=True)
            self.assertNotEqual(changed.returncode,0)
            self.assertIn('checksum mismatch',changed.stderr)
            (package/'src/lib.rs').write_text('fn broken() { missing_ci_fixture_function(); }')
            broken = subprocess.run(['python3',script,'compile',str(artifacts),'--',*build,'--lib'],
                                    cwd=package,env=env,capture_output=True,text=True)
            self.assertNotEqual(broken.returncode,0)
            self.assertIn('missing_ci_fixture_function',broken.stderr)

    def test_main_compilation_can_overlap_binary_build(self):
        producer = self.jobs['workflow-test-build']
        self.assertEqual(producer['needs'], 'changes')
        self.assertIn('workflow-test-build', self.jobs['workflow-test']['needs'])
        for tier in ['pr', 'main', 'nightly', 'full']:
            values = self.values(tier)
            self.assertEqual(evaluate(producer['if'], values),
                             evaluate(self.jobs['workflow-test']['if'], values))
        self.assertFalse(any('cargo test' in s.get('run', '')
                             for s in self.jobs['workflow-test']['steps']))
        self.assertIn('workflow-test-build', self.jobs['ci-success']['needs'])

    def test_archived_test_executable_receives_cargo_fixture_context(self):
        workflow=yaml.safe_load((ROOT/'.github/workflows/ci-test-archive.yml').read_text())
        step=next(s for s in workflow['jobs']['execute']['steps'] if s.get('name')=='Verify and run the shared executable')
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'crates/server').mkdir(parents=True)
            binary=root/'server-tests'
            binary.write_text('#!/bin/sh\nset -eu\ntest "$PWD" = "$GITHUB_WORKSPACE/crates/server"\ntest "$CARGO_MANIFEST_DIR" = "$PWD"\n')
            binary.chmod(0o755)
            with tarfile.open(root/'server-tests.tar.gz','w:gz') as archive:archive.add(binary,arcname='server-tests')
            (root/'server-tests.sha256').write_text(hashlib.sha256(binary.read_bytes()).hexdigest()+'  server-tests\n')
            binary.unlink()
            env=dict(os.environ,GITHUB_WORKSPACE=str(root));env.pop('CARGO_MANIFEST_DIR',None)
            env.update({key:value.replace('${{ github.workspace }}',str(root)) for key,value in step.get('env',{}).items()})
            if not shutil.which('sha256sum'):
                shim=root/'bin';shim.mkdir()
                (shim/'sha256sum').write_text('#!/bin/sh\nexec shasum -a 256 "$@"\n')
                (shim/'sha256sum').chmod(0o755)
                env['PATH']=str(shim)+os.pathsep+env['PATH']
            subprocess.run(['bash','-euo','pipefail','-c',step['run']],cwd=root,env=env,check=True,capture_output=True)

    def values(self, tier='pr'):
        return {'needs.changes.outputs.mode':tier,'needs.changes.outputs.run_ci':'true',
                'needs.changes.outputs.rust':'true','needs.changes.outputs.postgres_integration':'true',
                'needs.changes.outputs.has_tests':'true','needs.changes.outputs.offline':'true',
                'github.event_name':'pull_request' if tier=='pr' else 'push',
                'github.ref':'refs/pull/1/merge' if tier=='pr' else 'refs/heads/main'}

    def test_ordinary_pr_does_not_build_binaries_or_run_broad_matrix(self):
        values=self.values()
        self.assertTrue(evaluate(self.jobs['focused-rust']['if'],values))
        for name in ['unit-test','integration-test','worker-test','workflow-test','workflow-test-build','build-binaries','sdk-compat-test','openapi-check','docker-build','msrv','wasm-portability']:
            self.assertFalse(evaluate(self.jobs[name]['if'],values),name)

    def test_focused_tests_restore_main_produced_caches(self):
        cache=next(s for s in self.jobs['focused-rust']['steps'] if s.get('uses','').startswith('Swatinem/rust-cache@'))
        self.assertIn('integration-v2',cache['with']['shared-key'])
        self.assertIn('unit-v2',cache['with']['shared-key'])
        self.assertIs(cache['with']['save-if'],False)
        def cache_environment(job):
            return {key:value for key,value in self.jobs[job].get('env',{}).items()
                    if key.startswith(('CARGO','CC','CFLAGS','CXX','CMAKE','RUST'))}
        for job in ['unit-test','integration-test','workflow-test-build']:
            self.assertEqual(cache_environment(job),cache_environment('focused-rust'))
        execution=next(s for s in self.jobs['focused-rust']['steps'] if s.get('name')=='Test affected crates and persistence contracts')
        self.assertEqual(execution['env']['RUST_TEST_THREADS'],'4')

    def test_api_pr_keeps_compatibility_without_worker_cli_builds(self):
        values=self.values(); values['needs.changes.outputs.sdk_compat']='true'
        for name in ['build-binaries','sdk-compat-test','openapi-check']:
            self.assertTrue(evaluate(self.jobs[name]['if'],values),name)
        self.assertNotIn('build-binaries',self.jobs['openapi-check']['needs'])
        build=next(s for s in self.jobs['build-binaries']['steps'] if s.get('name')=='Build CI binaries')
        self.assertFalse(evaluate(build['env']['BUILD_WORKER'],values))

    def test_chaos_uses_shared_binaries_and_only_relevant_prs(self):
        values=self.values()
        values['needs.build-binaries.result']='success'
        self.assertFalse(evaluate(self.jobs['chaos-test']['if'],values))
        values['needs.changes.outputs.chaos']='true'
        self.assertTrue(evaluate(self.jobs['chaos-test']['if'],values))
        self.assertTrue(evaluate(self.jobs['build-binaries']['if'],values))
        build=next(s for s in self.jobs['build-binaries']['steps'] if s.get('name')=='Build CI binaries')
        self.assertTrue(evaluate(build['env']['BUILD_WORKER'],values))
        self.assertFalse(evaluate(build['env']['BUILD_CLI'],values))
        self.assertIn('chaos-test',self.jobs['ci-success']['needs'])
        self.assertFalse(any('cargo build' in s.get('run','') for s in self.jobs['chaos-test']['steps']))
        manual=yaml.safe_load((ROOT/'.github/workflows/chaos.yml').read_text())
        self.assertNotIn('pull_request',manual.get('on',manual.get(True)))

    def test_main_keeps_offline_coverage(self):
        values=self.values('batch')
        for name in ['unit-test','worker-test','integration-test','workflow-test','workflow-test-build','build-binaries']:
            self.assertTrue(evaluate(self.jobs[name]['if'],values),name)
        self.assertFalse(evaluate(self.jobs['live-provider-matrix']['if'],values))

    def test_full_forces_every_offline_surface(self):
        outputs=self.jobs['changes']['outputs']
        values={'steps.plan.outputs.force':'true','steps.plan.outputs.mode':'full'}
        for key in ['rust','sdk_compat','postgres_integration','object_storage','ui','docs','docker']:
            self.assertTrue(evaluate(outputs[key],values),key)
        self.assertTrue(evaluate(outputs['provider_live'],values))

    def test_full_manual_validation_includes_cli_workflows(self):
        values=self.values('full');values['github.event_name']='workflow_dispatch'
        self.assertTrue(evaluate(self.jobs['cli-e2e-test']['if'],values))

    def test_unchanged_nightly_runs_provider_health_without_offline_matrix(self):
        outputs=self.jobs['changes']['outputs'];values={'steps.plan.outputs.force':'false','steps.plan.outputs.mode':'nightly'}
        self.assertTrue(evaluate(outputs['provider_live'],values))
        self.assertFalse(evaluate(outputs['rust'],values))
        values=self.values('nightly'); values.update({'needs.changes.outputs.rust':'','needs.changes.outputs.offline':'false',
            'needs.changes.outputs.provider_live':'true','github.event_name':'schedule'})
        self.assertTrue(evaluate(self.jobs['live-provider-matrix']['if'],values))
        self.assertFalse(evaluate(self.jobs['workflow-test']['if'],values))
        self.assertFalse(evaluate(self.jobs['build-binaries']['if'],values))

    def test_service_only_for_production_configuration_and_durable_store(self):
        image=self.jobs['integration-test']['services']['postgres']['image']
        for tier,shard,expected in [('batch','server',False),('batch','domain',False),('batch','durable',True),('nightly','server',True),('full','domain',True)]:
            self.assertEqual(evaluate(image,{'needs.changes.outputs.mode':tier,'matrix.shard':shard}),expected)
        steps=self.jobs['integration-test']['steps']
        migrate=next(s for s in steps if s.get('name')=='Run migrations')
        self.assertFalse(evaluate(migrate['if'],{'needs.changes.outputs.mode':'batch','matrix.shard':'server'}))

    def test_release_rejects_other_sha_pr_and_ordinary_batch(self):
        def run(n,event='push',title='CI full a',branch='main',path='.github/workflows/ci.yml'):
            return dict(id=n,event=event,display_title=title,head_branch=branch,path=path)
        runs=[run(1),run(2,event='pull_request'),run(3,title='CI batch a'),run(4,title='CI full b'),run(5,branch='topic')]
        self.assertEqual(release.latest_full(runs,'a')['id'],1)
        self.assertIsNone(release.latest_full(runs,'unknown'))

    def test_release_discovery_stops_before_irrelevant_history(self):
        def run(n,revision='a',event='workflow_dispatch'):
            return dict(id=n,event=event,display_title='CI full '+revision,head_branch='main',path='.github/workflows/ci.yml')
        calls=[]
        def read(endpoint):
            calls.append(endpoint)
            if 'event=push' in endpoint:return {'workflow_runs':[run(250,event='push')]}
            return {'workflow_runs':[run(n,'other') for n in range(200,100,-1)]}
        self.assertEqual(release.find_full('o/r','a',read)['id'],250)
        self.assertEqual(len(calls),2)

    def test_release_discovery_finds_later_manual_attempt_on_older_sha(self):
        calls=[]
        def run(n,revision='other',event='workflow_dispatch'):
            return dict(id=n,event=event,display_title='CI full '+revision,head_branch='main',path='.github/workflows/ci.yml')
        def read(endpoint):
            calls.append(endpoint)
            if 'event=push' in endpoint:return {'workflow_runs':[run(50,'a','push')]}
            if endpoint.endswith('&page=1'):return {'workflow_runs':[run(n) for n in range(200,100,-1)]}
            return {'workflow_runs':[run(70,'a')]}
        self.assertEqual(release.find_full('o/r','a',read)['id'],70)
        self.assertEqual(len(calls),3)

    def test_release_history_budget_fails_closed(self):
        def read(endpoint):
            if 'event=push' in endpoint:return {'workflow_runs':[]}
            return {'workflow_runs':[dict(id=n,event='workflow_dispatch',display_title='unrelated',head_branch='main',path='.github/workflows/ci.yml') for n in range(200,100,-1)]}
        with self.assertRaises(SystemExit):release.find_full('o/r','a',read)

    def test_release_side_effects_follow_exact_revision_gate(self):
        for file,job,target in [('release.yml','release','Create GitHub Release'),('publish-crates.yml','publish','Publish package')]:
            workflow=yaml.safe_load((ROOT/'.github/workflows'/file).read_text());steps=workflow['jobs'][job]['steps']
            gate=next(i for i,s in enumerate(steps) if 'wait_full.py' in s.get('run',''))
            side=next(i for i,s in enumerate(steps) if s.get('name')==target)
            self.assertLess(gate,side,file)

    def test_failure_dedup_stale_event_and_recovery(self):
        state={'run_id':10,'failed_sha':'broken'}
        run={'id':11,'revision':'fix','conclusion':'success'}
        self.assertEqual(tracker.decision(state,run,{'conclusion':'success'},lambda a,b:True),'resolved')
        self.assertEqual(tracker.decision(state,dict(run,revision='broken'),{'conclusion':'success'},lambda a,b:True),'ignore')
        self.assertEqual(tracker.decision(state,dict(run,conclusion='failure'),{'conclusion':'success'},lambda a,b:True),'ignore')
        self.assertEqual(tracker.decision(state,dict(run,id=9),{'conclusion':'failure'},lambda a,b:True),'ignore')
        self.assertEqual(tracker.decision(state,run,{'conclusion':'skipped'},lambda a,b:True),'ignore')
        self.assertEqual(tracker.decision(None,run,{'conclusion':'failure'},lambda a,b:True),'failure')
        closed=dict(state,run_id=12,resolved=True)
        self.assertEqual(tracker.decision(closed,run,{'conclusion':'failure'},lambda a,b:True),'ignore')
        self.assertEqual(tracker.decision(closed,dict(run,id=13),{'conclusion':'failure'},lambda a,b:True),'failure')
        self.assertEqual(tracker.decision(closed,dict(run,id=13),{'conclusion':'success'},lambda a,b:True),'ignore')
        self.assertEqual(tracker.check_key('SDK Compat (rust / last_minor / v0.2.1)'),tracker.check_key('SDK Compat (rust / last_minor / v0.3.1)'))

    def test_generated_updates_preserve_human_investigation(self):
        old='Human analysis\n\n<!-- ci-report-start -->old<!-- ci-report-end -->\nFix plan'
        updated=tracker.replace_report(old,'new')
        self.assertIn('Human analysis',updated);self.assertIn('Fix plan',updated)
        self.assertNotIn('>old<',updated);self.assertIn('new',updated)

    def test_recovery_probe_refuses_non_durable_or_unowned_targets(self):
        with tempfile.TemporaryDirectory() as directory:
            docker=Path(directory)/'docker'
            docker.write_text('#!/usr/bin/env python3\nimport os,sys,json\nwith open(os.environ[\'DOCKER_LOG\'],\'a\') as f:f.write(json.dumps(sys.argv[1:])+\'\\n\')\nargs=\' \'.join(sys.argv)\nif "current_setting" in args:print(os.environ.get(\'DURABILITY\',\'t\'))\nelif "SELECT count(*)" in args:print(\'1\')\n')
            docker.chmod(0o755)
            log=Path(directory)/'log'
            env=dict(os.environ,PATH=directory+':'+os.environ['PATH'],DOCKER_LOG=str(log),PG_CONTAINER='a'*64)
            script=str(ROOT/'.github/scripts/ci/postgres_recovery.sh')
            result=subprocess.run(['bash',script],env=env,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            calls=[json.loads(x) for x in log.read_text().splitlines()]
            self.assertIn(['kill','--signal=KILL','a'*64],calls)
            self.assertIn(['start','a'*64],calls)
            log.write_text('')
            result=subprocess.run(['bash',script],env=dict(env,DURABILITY='f'),capture_output=True)
            self.assertNotEqual(result.returncode,0)
            self.assertNotIn('kill',log.read_text())
            log.write_text('')
            result=subprocess.run(['bash',script],env=dict(env,PG_CONTAINER='unrelated-name'),capture_output=True)
            self.assertNotEqual(result.returncode,0)
            self.assertEqual(log.read_text(),'')

    def test_every_consumer_checks_out_attested_revision(self):
        for key,job in self.jobs.items():
            if key=='changes':continue
            for step in job.get('steps',[]):
                if step.get('uses','').startswith('actions/checkout@'):
                    self.assertEqual(step['with']['ref'],'${{ needs.changes.outputs.revision }}',key)

    def test_archive_experiment_isolates_cache_and_measures_compression(self):
        workflow=yaml.safe_load((ROOT/'.github/workflows/ci-test-archive.yml').read_text())
        compile=workflow['jobs']['compile']; execute=workflow['jobs']['execute']
        self.assertEqual(compile['strategy']['matrix']['cache'],['dependencies','workspace'])
        self.assertEqual(execute['strategy']['matrix']['cache'],['dependencies','workspace'])
        cache=next(s for s in compile['steps'] if s.get('uses','').startswith('Swatinem/rust-cache@'))
        self.assertIn('${{ matrix.cache }}',cache['with']['shared-key'])
        self.assertIn('${{ github.sha }}',cache['with']['shared-key'])
        self.assertEqual(cache['with']['cache-workspace-crates'],"${{ matrix.cache == 'workspace' }}")
        upload=next(s for s in compile['steps'] if s.get('uses','').startswith('actions/upload-artifact@'))
        self.assertEqual(upload['with']['compression-level'],0)
        archive=next(s for s in compile['steps'] if s.get('name')=='Archive only the compiled test executable')
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); binary=root/'binary'
            binary.write_bytes(b'fixture payload\n'*1000);binary.chmod(0o755)
            artifacts=root/'artifacts.jsonl'
            artifacts.write_text(json.dumps(dict(reason='compiler-artifact',executable=str(binary),profile={'test':True}))+'\n')
            command=archive['run'].replace('/tmp/test-artifacts.jsonl',str(artifacts))
            subprocess.run(['bash','-euo','pipefail','-c',command],cwd=root,
                           env=dict(os.environ,GITHUB_SHA='fixture-sha'),check=True,capture_output=True)
            report=json.loads((root/'archive-measurements.json').read_text())
            self.assertEqual(report['revision'],'fixture-sha')
            self.assertEqual([r['level'] for r in report['gzip']],[1,9])
            self.assertEqual(report['binary_bytes'],binary.stat().st_size)
            with tarfile.open(root/'server-tests.tar.gz') as tar:
                self.assertEqual(tar.extractfile('server-tests').read(),binary.read_bytes())
            self.assertEqual((root/'server-tests.sha256').read_text().split()[0],hashlib.sha256(binary.read_bytes()).hexdigest())


    def test_existing_linker_avoids_package_manager_and_exposes_command(self):
        with tempfile.TemporaryDirectory(prefix='ci linker ') as directory:
            root = Path(directory)
            env = dict(os.environ, PATH=str(root), RUNNER_TEMP=str(root/'runner'),
                       GITHUB_PATH=str(root/'path'), CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS='-C link-arg=-fuse-ld=lld')
            for command in ('mkdir', 'ln'):
                (root/command).symlink_to(shutil.which(command))
            (root/'ld.lld-18').write_text('#!/bin/sh\necho "LLD 18 fixture"\n')
            (root/'ld.lld-18').chmod(0o755)
            command = ['/bin/bash', str(ROOT/'scripts/ci-prepare-linker.sh')]
            result = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('LLD 18 fixture', result.stdout)
            directory = Path((root/'path').read_text().strip())
            self.assertEqual((directory/'ld.lld').resolve(), (root/'ld.lld-18').resolve())
            self.assertIn('LLD 18 fixture', subprocess.check_output(['ld.lld', '--version'], env=dict(env, PATH=str(directory)), text=True))
            # A pre-existing unversioned linker requires neither an alias nor apt.
            (root/'path').unlink()
            (root/'ld.lld').symlink_to(root/'ld.lld-18')
            result = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse((root/'path').exists())
            # No installed linker: package-manager failures remain real failures.
            (root/'ld.lld').unlink(); (root/'ld.lld-18').unlink()
            (root/'sudo').write_text('#!/bin/sh\nexit 37\n'); (root/'sudo').chmod(0o755)
            self.assertEqual(subprocess.run(command, env=env, capture_output=True).returncode, 37)
        prepare = next(step for step in self.jobs['workflow-test-build']['steps'] if step.get('name') == 'Prepare linker and disk')
        self.assertIn('bash scripts/ci-prepare-disk.sh', prepare['run'])
        self.assertIn('bash scripts/ci-prepare-linker.sh', prepare['run'])
        self.assertNotIn('rm -rf', prepare['run'])
        self.assertEqual(prepare['timeout-minutes'], 5)

    def test_disk_preparation_keeps_headroom_without_unnecessary_deletion(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); log=root/'cleanup.log'
            (root/'df').write_text('#!/bin/sh\necho "Filesystem 1024-blocks Used Available Capacity Mounted"\necho "fixture 100000000 1 $FREE_KIB 1% /"\n')
            (root/'sudo').write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$CLEANUP_LOG"\n')
            for name in ['df','sudo']:(root/name).chmod(0o755)
            env=dict(os.environ,PATH=str(root)+os.pathsep+os.environ['PATH'],CLEANUP_LOG=str(log))
            command=['bash',str(ROOT/'scripts/ci-prepare-disk.sh')]
            threshold=50*1024*1024
            for available in [threshold,threshold+1]:
                result=subprocess.run(command,env=dict(env,FREE_KIB=str(available)),capture_output=True,text=True)
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertFalse(log.exists())
                self.assertIn('Skipping SDK cleanup',result.stdout)
            subprocess.run(command,env=dict(env,FREE_KIB=str(threshold-1)),check=True,capture_output=True)
            cleanup=log.read_text()
            for path in ['/usr/share/dotnet','/usr/local/lib/android','/opt/ghc']:self.assertIn('rm -rf '+path,cleanup)
            self.assertNotIn('CodeQL',cleanup)
            log.unlink()
            subprocess.run([*command,'--extended'],env=dict(env,FREE_KIB='1024'),check=True,capture_output=True)
            self.assertIn('/opt/hostedtoolcache/CodeQL',log.read_text())
            log.unlink()
            invalid=subprocess.run([*command,'/unrelated'],env=dict(env,FREE_KIB='1024'),capture_output=True)
            self.assertNotEqual(invalid.returncode,0)
            self.assertFalse(log.exists())
            unknown=subprocess.run(command,env=dict(env,FREE_KIB='unknown'),capture_output=True)
            self.assertNotEqual(unknown.returncode,0)
            self.assertFalse(log.exists())


if __name__=='__main__':
    unittest.main()
