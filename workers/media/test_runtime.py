import contextlib
import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import struct
import shutil
import subprocess
import tempfile
import threading
import unittest
import sys
from unittest.mock import patch
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import zlib

spec = importlib.util.spec_from_file_location('media', Path(__file__).with_name('runtime.py'))
media = importlib.util.module_from_spec(spec)
spec.loader.exec_module(media)
PROFILE = json.loads(Path(__file__).with_name('qwen-image-v1.json').read_text())
VIDEO_PROFILE = json.loads(Path(__file__).with_name('wan-video-v1.json').read_text())


class ContributionMemoryTests(unittest.TestCase):
    def test_floor_uses_exact_capped_bytes_and_preserves_profile_fit(self):
        gib = 1024**3
        self.assertEqual(media.MINIMUM_MEDIA_BUDGET, 24*gib)
        for total, cap, eligible in [(32,50,False),(32,74,False),(32,75,True),(48,50,True),(64,30,False),(64,40,True),(24,80,False)]:
            budget = media.contribution_budget(total*gib, cap)
            if eligible:
                media.require_media_budget(budget)
            else:
                with self.assertRaisesRegex(media.MediaError, 'after applying the cap'):
                    media.require_media_budget(budget)
        with self.assertRaises(media.MediaError):
            media.require_media_budget(media.contribution_budget(32*gib-1,75))
        for cap in [0,81,100]:
            with self.assertRaises(media.MediaError): media.contribution_budget(128*gib,cap)
        with self.assertRaisesRegex(media.MediaError, 'model profile'):
            media.require_media_budget(24*gib, PROFILE)

    def test_all_platforms_and_media_operations_reject_before_setup_or_network(self):
        for system in ['Darwin', 'Windows', 'Linux']:
            for operation in ['text_to_image','image_edit','text_to_video','image_to_video']:
                for action in ['setup','verify','generate']:
                    with self.subTest(system=system,operation=operation,action=action), tempfile.TemporaryDirectory() as directory:
                        home = Path(directory)
                        profile = {**PROFILE, 'operation': operation, 'minimum_budget_bytes': 0}
                        path = home/'profile.json'; path.write_text(json.dumps(profile))
                        args = ['runtime.py',action,'--home',directory,'--profile',str(path),'--cap-percent','50','--endpoint','http://127.0.0.1:8188','--yes']
                        with patch.object(sys,'argv',args), patch.object(media.platform,'system',return_value=system), \
                             patch.object(media,'physical_memory',return_value=32*1024**3), \
                             patch.object(media,'Comfy') as comfy, patch.object(media,'install') as install:
                            with self.assertRaisesRegex(media.MediaError,'24 GiB'): media.main()
                            comfy.assert_not_called(); install.assert_not_called()
                        self.assertFalse((home/'media').exists())

    def test_plan_explains_ineligibility_and_stop_still_works(self):
        with tempfile.TemporaryDirectory() as directory:
            args = ['runtime.py','plan','--home',directory,'--profile',str(Path(__file__).with_name('qwen-image-v1.json')),'--cap-percent','50']
            output = io.StringIO()
            with patch.object(sys,'argv',args), patch.object(media,'physical_memory',return_value=32*1024**3), contextlib.redirect_stdout(output):
                media.main()
            plan = json.loads(output.getvalue())
            self.assertFalse(plan['media_eligible'])
            self.assertEqual(plan['budget_bytes'],16*1024**3)
            args[1]='stop'
            with patch.object(sys,'argv',args), patch.object(media,'physical_memory',side_effect=media.MediaError('unavailable')) as probe, \
                 patch.object(media,'stop_owned_container') as stop, contextlib.redirect_stdout(io.StringIO()):
                media.main()
                probe.assert_not_called(); stop.assert_called_once()

    def test_status_invalidates_matching_certificate_below_floor(self):
        with tempfile.TemporaryDirectory() as directory:
            home=Path(directory)
            certificate={'ready':True,'profile_hash':media.profile_hash(PROFILE),'cap_percent':50,'endpoint':None}
            media.atomic_json(home/'media/verified.json',certificate)
            args=['runtime.py','status','--home',directory,'--profile',str(Path(__file__).with_name('qwen-image-v1.json')),'--cap-percent','50']
            output=io.StringIO()
            with patch.object(sys,'argv',args), patch.object(media,'physical_memory',return_value=32*1024**3), contextlib.redirect_stdout(output):
                media.main()
            self.assertFalse(json.loads(output.getvalue())['verified']['ready'])


def png(width=8, height=8):
    def chunk(kind, content):
        return struct.pack('>I', len(content))+kind+content+struct.pack('>I',zlib.crc32(kind+content)&0xffffffff)
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,2,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+b'\xff\0\0'*width)*height))+chunk(b'IEND',b'')


class FakeComfy:
    def __init__(self, profile=PROFILE, output=None):
        self.profile=profile; self.output=output
        self.requests=[]; self.busy=False; self.error=False; self.bad_image=False; self.pending=False; self.token=None
    def __enter__(self):
        fixture=self
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args): pass
            def do_GET(self): self.respond(None)
            def do_POST(self): self.respond(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
            def respond(self,body):
                fixture.requests.append((self.command,self.path,body))
                if self.path=='/system_stats': result={'system':{'comfyui_version':'fixture'},'devices':[]}
                elif self.path=='/object_info':
                    result={node['class_type']:{'input':{'required':{}}} for node in media.workflow(fixture.profile,'test',1,'test').values()}
                    for kind,key,index in [('UNETLoader','unet_name',0),('CLIPLoader','clip_name',1),('VAELoader','vae_name',2)]:
                        result[kind]['input']['required'][key]=[[Path(fixture.profile['files'][index]['path']).name]]
                    if fixture.profile.get('architecture') == 'wan22_t2v_a14b':
                        result['UNETLoader']['input']['required']['unet_name'][0].append(Path(fixture.profile['files'][3]['path']).name)
                elif self.path=='/queue': result={'queue_running':[1] if fixture.busy else [],'queue_pending':[]}
                elif self.path=='/prompt':
                    fixture.token=body['client_id'];result={'prompt_id':'owned-job','node_errors':{}}
                elif self.path.startswith('/history/'):
                    ext='.mp4' if media.is_video(fixture.profile) else '.png'
                    result={} if fixture.pending else {'owned-job':{'status':{'completed':True,'status_str':'error' if fixture.error else 'success'},'outputs':{'10':{'images':[{'filename':fixture.token+'_00001_'+ext,'subfolder':'opengpu','type':'output'}]}}}}
                elif self.path.startswith('/view?'): result=b'bad' if fixture.bad_image else (fixture.output if fixture.output is not None else png())
                else: self.send_error(404);return
                raw=result if isinstance(result,bytes) else json.dumps(result).encode()
                self.send_response(200);self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
        self.server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True);self.thread.start()
        self.client=media.Comfy('http://127.0.0.1:'+str(self.server.server_port))
        return self
    def __exit__(self,*args): self.server.shutdown();self.server.server_close();self.thread.join()


class RuntimeTests(unittest.TestCase):
    def test_interactive_download_bar_preserves_machine_output_mode(self):
        data = b'weights'
        checksum = hashlib.sha256(data).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            output, terminal = io.StringIO(), io.StringIO()
            with patch.dict(media.os.environ, {'OPENGPU_MEDIA_HUMAN_PROGRESS': '1'}), contextlib.redirect_stdout(output), contextlib.redirect_stderr(terminal), patch.object(terminal, 'isatty', return_value=True), patch.object(media.urllib.request, 'urlopen', return_value=io.BytesIO(data)):
                media.download('https://example.test/model', Path(directory)/'model', len(data), checksum)
            self.assertIn('[########################] 100%', terminal.getvalue())
            self.assertIn('Download checksum verified', terminal.getvalue())
            self.assertEqual(output.getvalue(), '')
        output = io.StringIO()
        with patch.dict(media.os.environ, {'OPENGPU_MEDIA_HUMAN_PROGRESS': '0'}), contextlib.redirect_stdout(output):
            media.emit('download_progress', downloaded_bytes=7, total_bytes=7)
        self.assertEqual(json.loads(output.getvalue())['downloaded_bytes'], 7)

    def test_download_progress_and_verified_cache_reuse(self):
        data = b'model bytes'
        checksum = hashlib.sha256(data).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'model.bin'
            with patch.object(media.urllib.request, 'urlopen', return_value=io.BytesIO(data)) as fetch, patch.object(media, 'emit') as emit:
                media.download('https://example.test/model', path, len(data), checksum)
                progress = [call.kwargs for call in emit.call_args_list if call.args == ('download_progress',)]
                self.assertEqual(progress[-1]['downloaded_bytes'], len(data))
                self.assertEqual(progress[-1]['total_bytes'], len(data))
                media.download('https://example.test/model', path, len(data), checksum)
                self.assertEqual(fetch.call_count, 1)
                self.assertEqual(path.read_bytes(), data)

    def test_cache_space_check_rejects_corrupt_same_size_model(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'models' / 'model.bin'
            path.parent.mkdir()
            path.write_bytes(b'good')
            file = {'path': 'model.bin', 'size': 4, 'sha256': hashlib.sha256(b'good').hexdigest()}
            self.assertTrue(media.cached_model(root, file))
            path.write_bytes(b'bad!')
            self.assertFalse(media.cached_model(root, file))

    def test_14b_workflow_preserves_latent_between_noise_experts(self):
        graph = media.workflow(VIDEO_PROFILE, 'a car', 42, 'test')
        self.assertIn('high_noise_14B', graph['1']['inputs']['unet_name'])
        self.assertIn('low_noise_14B', graph['12']['inputs']['unet_name'])
        high, low = graph['8']['inputs'], graph['14']['inputs']
        self.assertEqual(high['end_at_step'], low['start_at_step'])
        self.assertEqual(low['latent_image'], ['8', 0])
        self.assertEqual(high['return_with_leftover_noise'], 'enable')
        self.assertEqual(low['add_noise'], 'disable')
        self.assertEqual(graph['9']['inputs']['samples'], ['14', 0])
        self.assertEqual(graph['6']['class_type'], 'EmptyHunyuanLatentVideo')
        self.assertEqual(media.preset_profile(VIDEO_PROFILE, 10)['id'], 'wan22-14b-704p-161f-v1')

    def setUp(self):
        self.profile=copy.deepcopy(PROFILE);self.profile['width']=8;self.profile['height']=8

    def test_existing_comfy_execution_downloads_and_validates_owned_image(self):
        with tempfile.TemporaryDirectory() as directory, FakeComfy() as fixture, contextlib.redirect_stdout(io.StringIO()):
            output=Path(directory)/'image.png'
            result=media.generate(fixture.client,self.profile,'a teapot',42,output)
            self.assertEqual(output.read_bytes(),png())
            self.assertEqual(result['sha256'],hashlib.sha256(png()).hexdigest())
            posts=[route for method,route,_ in fixture.requests if method=='POST']
            self.assertEqual(posts,['/prompt'])

    def test_busy_external_runtime_is_not_interrupted(self):
        with FakeComfy() as fixture:
            fixture.busy=True
            with self.assertRaisesRegex(media.MediaError,'busy'):
                media.generate(fixture.client,self.profile,'a teapot',42,Path('unused.png'))
            self.assertFalse(any(method=='POST' for method,_,_ in fixture.requests))

    def test_timeout_cancels_only_our_pending_job(self):
        with FakeComfy() as fixture, contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(media.MediaError,'timed out'):
                media.generate(fixture.client,self.profile,'a teapot',42,Path('unused.png'),timeout=0)
            self.assertIn(('POST','/queue',{'delete':['owned-job']}),fixture.requests)
            self.assertFalse(any(route=='/interrupt' for _,route,_ in fixture.requests))

    def test_execution_error_and_invalid_image_do_not_create_artifact(self):
        for field in ['error','bad_image']:
            with tempfile.TemporaryDirectory() as directory, FakeComfy() as fixture, contextlib.redirect_stdout(io.StringIO()):
                setattr(fixture,field,True);output=Path(directory)/'image.png'
                with self.assertRaises(media.MediaError):media.generate(fixture.client,self.profile,'teapot',42,output)
                self.assertFalse(output.exists())

    def test_png_integrity_and_dimensions(self):
        media.validate_png(png(),8,8)
        for data,width,height in [(png(),16,8),(png()[:-1],8,8),(png()+b'x',8,8),(png().replace(b'IDAT',b'IDXT'),8,8)]:
            with self.assertRaises(media.MediaError):media.validate_png(data,width,height)

    def test_workflow_pins_model_settings_and_treats_prompt_as_data(self):
        prompt='"}; arbitrary-node; {'
        value=media.workflow(PROFILE,prompt,123,'owned')
        self.assertEqual(value['4']['inputs']['text'],prompt)
        self.assertEqual(value['8']['inputs']['steps'],30)
        self.assertEqual(value['6']['inputs']['batch_size'],1)
        self.assertEqual(value['10']['inputs']['filename_prefix'],'opengpu/owned')
        with self.assertRaises(media.MediaError):media.workflow(PROFILE,'',1,'owned')

    def test_download_verifies_before_replacing_existing_model(self):
        with tempfile.TemporaryDirectory() as directory, contextlib.redirect_stdout(io.StringIO()):
            path=Path(directory)/'model';path.write_bytes(b'old')
            with patch.object(media.urllib.request,'urlopen',return_value=io.BytesIO(b'new')):
                with self.assertRaisesRegex(media.MediaError,'checksum'):
                    media.download('https://example.test/model',path,3,'0'*64)
            self.assertEqual(path.read_bytes(),b'old')

    def test_drain_requires_matching_agent_ack_and_cleans_up(self):
        with tempfile.TemporaryDirectory() as directory, contextlib.redirect_stdout(io.StringIO()):
            home=Path(directory);media.atomic_json(home/'config.json',{'connected':True,'paused':False})
            media.atomic_json(home/'media/drain-ack.json',{'id':'old','drained':True})
            with self.assertRaisesRegex(media.MediaError,'not acknowledged'):
                with media.drain_contributor(home,timeout=0):self.fail('must not enter')
            self.assertFalse((home/'media/drain-request.json').exists())
            def acknowledge(_):
                value=media.load_json(home/'media/drain-request.json');value['drained']=True
                media.atomic_json(home/'media/drain-ack.json',value)
            with patch.object(media.time,'sleep',side_effect=acknowledge):
                with media.drain_contributor(home,timeout=1):
                    self.assertTrue((home/'media/drain-request.json').exists())
            self.assertFalse((home/'media/drain-request.json').exists())

    def test_paused_but_busy_agent_does_not_allow_verification(self):
        with tempfile.TemporaryDirectory() as directory:
            home=Path(directory);media.atomic_json(home/'config.json',{'connected':True,'paused':True})
            media.atomic_json(home/'agent-state.json',{'agent_state':'busy'})
            with self.assertRaises(media.MediaError):
                with media.drain_contributor(home):self.fail('must not enter')

    def test_native_backend_selection(self):
        for system, machine, expected in [('Windows','AMD64','cuda'), ('Darwin','arm64','mps'),
                                          ('Darwin','x86_64',None), ('Linux','aarch64',None)]:
            with patch.object(media.platform,'system',return_value=system), patch.object(media.platform,'machine',return_value=machine):
                self.assertEqual(media.native_backend(),expected)

    def test_native_install_is_isolated_probes_gpu_before_model_download_and_needs_verification(self):
        for backend in ['cuda','mps']:
            with tempfile.TemporaryDirectory() as directory, contextlib.redirect_stdout(io.StringIO()):
                root=Path(directory); source=root/'source'; source.mkdir()
                commands=[]; downloads=[]
                def command(args, **kwargs): commands.append(args); return ''
                def download(*args):
                    self.assertTrue(any('-c' in cmd for cmd in commands))
                    downloads.append(args)
                with patch.object(media,'native_backend',return_value=backend), \
                     patch.object(media.shutil,'which',return_value='nvidia-smi'), \
                     patch.object(media.shutil,'disk_usage',return_value=type('Disk',(),{'free':10**12})()), \
                     patch.object(media,'cached_model',return_value=True), \
                     patch.object(media,'prepare_source',return_value=source), \
                     patch.object(media,'run',side_effect=command), \
                     patch.object(media,'download',side_effect=download), \
                     patch.object(media,'drain_contributor',return_value=contextlib.nullcontext()), \
                     patch.object(media.sys,'version_info',(3,13,0)):
                    media.install_native(root,PROFILE,64*1024**3)
                self.assertEqual(commands[0][1:3],['-m','venv'])
                self.assertIn('torch==2.9.1',commands[1])
                self.assertEqual('--index-url' in commands[1],backend=='cuda')
                self.assertIn('-c',commands[2]); self.assertIn('-r',commands[2])
                self.assertEqual(len(downloads),len(PROFILE['files']))
                record=media.load_json(media.profile_record(root,PROFILE,'runtime'))
                self.assertEqual(record['kind'],'managed_native_comfyui')
                self.assertEqual(record['backend'],backend)
                self.assertFalse(media.profile_record(root,PROFILE,'verified').exists())

    def test_native_missing_gpu_fails_before_model_download(self):
        with patch.object(media,'native_backend',return_value='cuda'), \
             patch.object(media.shutil,'which',return_value=None), patch.object(media,'download') as download:
            with self.assertRaisesRegex(media.MediaError,'NVIDIA'):
                media.install_native(Path('.'),PROFILE,64*1024**3)
            download.assert_not_called()

    def test_native_runtime_uses_owned_paths_loopback_and_stops_on_generation_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); base,python=media.native_paths(root,PROFILE)
            python.parent.mkdir(parents=True); python.touch()
            source=root/('build-'+PROFILE['comfy_revision'])/('ComfyUI-'+PROFILE['comfy_revision'])
            source.mkdir(parents=True); (source/'main.py').touch()
            calls=[]
            class Process:
                pid=123; stopped=False
                def poll(self): return 0 if self.stopped else None
                def terminate(self): self.stopped=True
                def wait(self,timeout): return 0
            process=Process()
            def launch(args,**kwargs): calls.append((args,kwargs)); return process
            class Client:
                def __init__(self,url): self.url=url
                def inspect(self,profile): return {},{}
            record={'backend':'mps'}
            with patch.object(media,'native_backend',return_value='mps'), \
                 patch.object(media,'physical_memory',return_value=128*1024**3), \
                 patch.object(media.subprocess,'Popen',side_effect=launch), patch.object(media,'Comfy',Client):
                with self.assertRaisesRegex(RuntimeError,'generation failed'):
                    with media.native_client(root,PROFILE,record,64*1024**3) as client:
                        self.assertTrue(client.url.startswith('http://127.0.0.1:'))
                        self.assertTrue((root/'active-native.json').exists())
                        raise RuntimeError('generation failed')
            self.assertTrue(process.stopped); self.assertFalse((root/'active-native.json').exists())
            args,kwargs=calls[0]
            self.assertIn('--disable-all-custom-nodes',args)
            self.assertIn('--fp16-unet',args)
            self.assertIn('--fp16-text-enc',args)
            self.assertEqual(args[args.index('--output-directory')+1],str(root/'outputs'))
            self.assertEqual(args[args.index('--listen')+1],'127.0.0.1')
            self.assertEqual(args[args.index('mps')+1],'0.5')

    def test_invalid_native_cleanup_marker_never_runs_a_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); media.atomic_json(root/'active-native.json',{'pid':1,'token':'invalid','python':'outside'})
            with patch.object(media,'run') as run:
                with self.assertRaisesRegex(media.MediaError,'ownership'):
                    media.stop_owned_native(root)
                run.assert_not_called()
            self.assertTrue((root/'active-native.json').exists())

    def test_unsupported_platform_does_not_install_dependencies(self):
        with patch.object(media.platform,'system',return_value='Darwin'),patch.object(media.platform,'machine',return_value='x86_64'),patch.object(media,'run') as run:
            with self.assertRaisesRegex(media.MediaError,'Linux ARM64'):
                media.install(Path('.'),PROFILE,64*1024**3)
            run.assert_not_called()

    def test_cleanup_only_stops_owned_container_and_preserves_marker_on_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            name='opengpu-media-'+'a'*32
            owner=media.hashlib.sha256(str(root.resolve()).encode()).hexdigest()
            marker=root/'active-container.json'
            media.atomic_json(marker,{'name':name,'owner':owner})
            with patch.object(media,'run',side_effect=[name,'wrong-owner']) as command:
                with self.assertRaisesRegex(media.MediaError,'ownership'):
                    media.stop_owned_container(root)
                self.assertEqual(command.call_count,2)
            self.assertTrue(marker.exists())
            with patch.object(media,'run',side_effect=[name,owner,'']) as command:
                media.stop_owned_container(root)
                self.assertEqual(command.call_args.args[0],['docker','stop','--time','10',name])
            self.assertFalse(marker.exists())


class VideoTests(unittest.TestCase):
    def test_long_presets_keep_base_runtime_and_separate_certificates(self):
        original = copy.deepcopy(VIDEO_PROFILE)
        for seconds, frames in [(2, 33), (5, 81), (10, 161)]:
            profile = media.preset_profile(VIDEO_PROFILE, seconds)
            self.assertEqual(profile['frames'], frames)
            self.assertEqual(profile['files'], original['files'])
            self.assertEqual(media.workflow(profile, 'scene', 42, 'owned')['6']['inputs']['length'], frames)
            if seconds != 2:
                self.assertNotEqual(media.profile_record(Path('.'), profile, 'verified'),
                                    media.profile_record(Path('.'), original, 'verified'))
        self.assertEqual(VIDEO_PROFILE, original)
        with self.assertRaises(media.MediaError): media.preset_profile(VIDEO_PROFILE, 20)

    def test_workflow_is_bounded_and_uses_native_wan_nodes(self):
        flow=media.workflow(VIDEO_PROFILE,'A teapot slowly rotates',42,'owned')
        self.assertEqual(flow['6']['inputs']['length'],33)
        self.assertEqual(flow['6']['inputs']['width'],1280)
        self.assertEqual(flow['8']['inputs']['steps'],20)
        self.assertEqual(flow['10']['inputs']['codec'],'h264')
        self.assertEqual(flow['10']['inputs']['format'],'auto')
        self.assertEqual(flow['11']['inputs']['fps'],16)
        self.assertNotIn('start_image',flow['6']['inputs'])
        self.assertNotEqual(media.profile_record(Path('.'),PROFILE,'verified'), media.profile_record(Path('.'),VIDEO_PROFILE,'verified'))

    @unittest.skipUnless(shutil.which('ffmpeg') and shutil.which('ffprobe'),'ffmpeg and ffprobe required')
    def test_video_decode_frame_count_and_http_output_validation(self):
        profile=copy.deepcopy(VIDEO_PROFILE);profile.update(width=32,height=32,frames=5)
        with tempfile.TemporaryDirectory() as directory, contextlib.redirect_stdout(io.StringIO()):
            root=Path(directory); video=root/'fixture.mp4'
            subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i',f"color=c=red:s=32x32:r={profile['fps']}",
                '-frames:v','5','-c:v','libx264','-pix_fmt','yuv420p',str(video)],check=True,capture_output=True)
            media.validate_video(video,profile)
            for changes in [{'frames':9},{'width':64},{'fps':12}]:
                with self.assertRaises(media.MediaError):media.validate_video(video,{**profile,**changes})
            with FakeComfy(profile,video.read_bytes()) as fixture:
                result=media.generate(fixture.client,profile,'A teapot rotates',42,root/'valid.mp4')
                self.assertEqual(result['sha256'],media.digest_file(video))
                fixture.bad_image=True
                with self.assertRaises(media.MediaError):media.generate(fixture.client,profile,'test',42,root/'bad.mp4')
                self.assertFalse((root/'bad.mp4').exists())
                self.assertFalse(list(root.glob('*.partial.mp4')))

    def test_missing_video_decoder_prevents_gpu_submission(self):
        with FakeComfy(VIDEO_PROFILE) as fixture, patch.object(media.shutil,'which',return_value=None):
            with self.assertRaisesRegex(media.MediaError,'ffprobe'):
                media.generate(fixture.client,VIDEO_PROFILE,'test',42,Path('unused.mp4'))
            self.assertEqual(fixture.requests,[])


if __name__=='__main__':unittest.main()
