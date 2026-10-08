import contextlib
import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import time
import os
import unittest
from unittest.mock import Mock, patch

from workers.media.test_runtime import FakeComfy, PROFILE, VIDEO_PROFILE

spec = importlib.util.spec_from_file_location('persistent_media', Path(__file__).with_name('runtime.py'))
media = importlib.util.module_from_spec(spec)
spec.loader.exec_module(media)
real_owner = media.persistent_owner
spec = importlib.util.spec_from_file_location('media_bootstrap', Path(__file__).with_name('unified_memory.py'))
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


class PersistentRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        self.fixture = self.stack.enter_context(FakeComfy())
        self.stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
        self.owner = '0123456789abcdef0123456789abcdef'
        self.owner_live = True
        self.stack.enter_context(patch.dict(media.os.environ, {'OPENGPU_MEDIA_PERSISTENT_OWNER': self.owner}))
        self.stack.enter_context(patch.object(media, 'persistent_owner', side_effect=lambda root: self.owner if self.owner_live else None))
        self.stack.enter_context(patch.object(media, 'container_client', return_value=self.fixture.client))
        self.stack.enter_context(patch.object(media, 'run', side_effect=self.docker))
        self.containers = set()
        self.events = []
        self.actual_owner = hashlib.sha256(str(self.root.resolve()).encode()).hexdigest()
        self.profile = copy.deepcopy(PROFILE)
        self.profile.update(width=8, height=8)
        self.install(self.profile)

    def install(self, profile):
        media.atomic_json(media.profile_record(self.root, profile, 'runtime'),
                          {'profile_hash': media.profile_hash(profile), 'image_id': 'sha256:owned'})

    def docker(self, args, **kwargs):
        if args[:2] == ['docker', 'run']:
            name = args[args.index('--name') + 1]
            self.containers.add(name)
            self.events.append(('start', name))
            self.assertIn('OPENGPU_MEDIA_OWNER_INSTANCE=' + self.owner, args)
            return name
        if args[:2] == ['docker', 'ps']:
            return '\n'.join(self.containers)
        if args[:2] == ['docker', 'inspect']:
            if args[2] not in self.containers: raise media.MediaError('Container not found')
            return 'sha256:owned' if args[-1] == '{{.Image}}' else self.actual_owner
        if args[:2] == ['docker', 'stop']:
            self.containers.remove(args[-1])
            self.events.append(('stop', args[-1]))
            return args[-1]
        self.fail('Unexpected Docker command: ' + repr(args))

    def test_different_prompts_reuse_one_container_and_keep_outputs(self):
        infos = []
        for index, prompt in enumerate(['A red teapot', 'A blue teapot']):
            with media.client_for(self.root, self.profile, None, 1024**3) as client:
                infos.append(dict(client.runtime_info))
                media.generate(client, self.profile, prompt, index, self.root/f'{index}.png')
        self.assertFalse(infos[0]['runtime_reused'])
        self.assertTrue(infos[1]['runtime_reused'])
        self.assertEqual(len(self.events), 1)
        self.assertTrue((self.root/'0.png').is_file())
        self.assertTrue((self.root/'1.png').is_file())

    def test_model_switch_stops_previous_before_starting_video(self):
        with media.client_for(self.root, self.profile, None, 1024**3): pass
        self.install(VIDEO_PROFILE)
        with media.client_for(self.root, VIDEO_PROFILE, None, 1024**3): pass
        self.assertEqual([event[0] for event in self.events], ['start', 'stop', 'start'])

    def test_cap_change_restarts_runtime(self):
        with media.client_for(self.root, self.profile, None, 1024**3): pass
        with media.client_for(self.root, self.profile, None, 2*1024**3): pass
        self.assertEqual([event[0] for event in self.events], ['start', 'stop', 'start'])

    def test_crashed_container_is_recreated(self):
        with media.client_for(self.root, self.profile, None, 1024**3): pass
        self.containers.clear()
        with media.client_for(self.root, self.profile, None, 1024**3) as client:
            self.assertFalse(client.runtime_info['runtime_reused'])
        self.assertEqual(len(self.containers), 1)

    def test_failed_warm_job_clears_runtime_for_next_request(self):
        with media.client_for(self.root, self.profile, None, 1024**3): pass
        with self.assertRaises(media.MediaError):
            with media.client_for(self.root, self.profile, None, 1024**3):
                raise media.MediaError('Sampler failed')
        self.assertFalse((self.root/'active-container.json').exists())
        self.assertFalse(self.containers)

    def test_revoked_owner_cleans_up_and_cannot_start_orphaned_generation(self):
        with media.client_for(self.root, self.profile, None, 1024**3):
            self.owner_live = False
        self.assertFalse(self.containers)
        with self.assertRaisesRegex(media.MediaError, 'lease expired'):
            with media.client_for(self.root, self.profile, None, 1024**3): pass
        self.assertFalse(self.containers)

    def test_ownership_mismatch_never_stops_foreign_container(self):
        with media.client_for(self.root, self.profile, None, 1024**3): pass
        self.actual_owner = 'foreign-owner'
        with self.assertRaisesRegex(media.MediaError, 'ownership mismatch'):
            with media.client_for(self.root, self.profile, None, 1024**3): pass
        self.assertEqual(len(self.events), 1)

    def test_external_endpoint_does_not_start_or_stop_managed_runtime(self):
        with media.client_for(self.root, self.profile, self.fixture.client.endpoint, 1024**3) as client:
            self.assertEqual(client.endpoint, self.fixture.client.endpoint)
        self.assertFalse(self.events)

    @unittest.skipUnless(media.platform.system() == 'Linux', 'Linux worker flock interoperability')
    def test_owner_requires_a_live_worker_lock_and_unexpired_lease(self):
        import fcntl
        (self.root/'lifecycle').mkdir()
        ticks = Path(f'/proc/{os.getpid()}/stat').read_text().rsplit(') ',1)[1].split()[19]
        media.atomic_json(self.root/'lifecycle/lease.json', {'owner':self.owner, 'expires_at':time.time()+120,
                         'pid':os.getpid(), 'start_ticks':ticks})
        with (self.root/'worker.lock').open('w+b') as worker:
            fcntl.flock(worker, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(real_owner(self.root), self.owner)
        self.assertIsNone(real_owner(self.root))


class LeaseWatchdogTests(unittest.TestCase):
    def test_expired_mismatched_missing_and_malformed_leases_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'lease.json'
            owner = '0'*32
            self.assertFalse(bootstrap.lease_valid(path, owner, now=100))
            path.write_text(json.dumps({'owner':owner, 'expires_at':120}))
            self.assertTrue(bootstrap.lease_valid(path, owner, now=100))
            self.assertFalse(bootstrap.lease_valid(path, '1'*32, now=100))
            self.assertFalse(bootstrap.lease_valid(path, owner, now=120))
            path.write_text('invalid')
            self.assertFalse(bootstrap.lease_valid(path, owner, now=100))

    def test_watchdog_terminates_runtime_on_lease_loss(self):
        stop, terminate = Mock(), Mock()
        stop.wait.return_value = False
        with patch.object(bootstrap, 'lease_valid', return_value=False), contextlib.redirect_stdout(io.StringIO()):
            bootstrap.watch_lease('lease.json', '0'*32, stop, terminate)
        terminate.assert_called_once_with(75)


if __name__ == '__main__':
    unittest.main()
