#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import re
import unittest
import os
import subprocess
import tempfile
import json

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
    expression = re.sub(r'\b(?:github|needs|steps|matrix)(?:\.[a-zA-Z_][a-zA-Z_0-9]*)+',
                        lambda m: repr(values.get(m.group(), '')), expression)
    return bool(eval(expression.replace('&&',' and ').replace('||',' or '), {'__builtins__':{}}, {'always':lambda:True}))


class Policy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = yaml.safe_load((ROOT/'.github/workflows/ci.yml').read_text())
        cls.jobs = cls.workflow['jobs']

    def test_baseline_never_advances_on_failure_cancel_or_pr(self):
        runs = [dict(id=n,event=e,conclusion=c,head_branch='main',head_sha=sha,display_title='CI batch '+sha)
                for n,e,c,sha in [(1,'push','success','good'),(2,'push','failure','failed'),
                                   (3,'push','cancelled','cancelled'),(4,'pull_request','success','pr'),
                                   (5,'push',None,'running')]]
        self.assertEqual(plan.successful_base(runs,'now',lambda a,b:True),'good')
        self.assertEqual(plan.successful_base(runs,'now',lambda a,b:False),'')

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
        self.assertIn('migration_history_test::',focused.risk_filters(['crates/server/migrations/20261014000000_test.sql']))
        self.assertEqual(focused.risk_filters(['crates/core/src/engine.rs']),[])
        self.assertIn('service_mcp_oauth_lifecycle_test::', focused.risk_filters(['crates/server/tests/test_harness.rs']))

    def values(self, tier='pr'):
        return {'needs.changes.outputs.mode':tier,'needs.changes.outputs.run_ci':'true',
                'needs.changes.outputs.rust':'true','needs.changes.outputs.postgres_integration':'true',
                'needs.changes.outputs.has_tests':'true','needs.changes.outputs.offline':'true',
                'github.event_name':'pull_request' if tier=='pr' else 'push',
                'github.ref':'refs/pull/1/merge' if tier=='pr' else 'refs/heads/main'}

    def test_ordinary_pr_does_not_build_binaries_or_run_broad_matrix(self):
        values=self.values()
        self.assertTrue(evaluate(self.jobs['focused-rust']['if'],values))
        for name in ['unit-test','integration-test','worker-test','workflow-test','build-binaries','sdk-compat-test','openapi-check','docker-build','msrv','wasm-portability']:
            self.assertFalse(evaluate(self.jobs[name]['if'],values),name)

    def test_api_pr_keeps_compatibility_without_worker_cli_builds(self):
        values=self.values(); values['needs.changes.outputs.sdk_compat']='true'
        for name in ['build-binaries','sdk-compat-test','openapi-check']:
            self.assertTrue(evaluate(self.jobs[name]['if'],values),name)
        self.assertNotIn('build-binaries',self.jobs['openapi-check']['needs'])
        build=next(s for s in self.jobs['build-binaries']['steps'] if s.get('name')=='Build CI binaries')
        self.assertFalse(evaluate(build['env']['ALL_BINARIES'],values))

    def test_main_keeps_offline_coverage(self):
        values=self.values('batch')
        for name in ['unit-test','worker-test','integration-test','workflow-test','build-binaries']:
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


if __name__=='__main__':
    unittest.main()
