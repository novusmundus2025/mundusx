import contextlib
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock

from test_edit_quality import EDIT, options
from test_runtime import media, png


class ReferenceImageTests(unittest.TestCase):
    def test_reference_is_conditioning_only_and_primary_owns_output(self):
        graph = media.workflow(EDIT, 'Use the reference palette', 42, 'owned', 'scene.png', 'palette.png')
        self.assertEqual(graph['16']['inputs']['image'], 'palette.png')
        for key in ('4', '5'):
            self.assertEqual(graph[key]['inputs']['image1'], ['12', 0])
            self.assertEqual(graph[key]['inputs']['image2'], ['16', 0])
        self.assertEqual(graph['6']['inputs']['pixels'], ['12', 0])
        self.assertNotIn('16', media.workflow(EDIT, 'edit', 42, 'owned', 'scene.png'))
        with self.assertRaises(media.MediaError):
            media.workflow({**EDIT, 'operation':'image_to_video'}, 'edit', 42, 'owned', 'scene.png', 'reference.png')

    def test_reference_validation_precedes_renderer_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            scene = Path(directory)/'scene.png'; scene.write_bytes(png())
            bad = Path(directory)/'bad.png'; bad.write_bytes(b'not a png')
            client = MagicMock()
            with self.assertRaises(media.MediaError):
                media.generate(client, EDIT, 'edit', 42, Path(directory)/'out.png', input_image=scene, reference_image=bad)
            client.inspect.assert_not_called(); client.upload_image.assert_not_called()
            with self.assertRaises(media.MediaError):
                media.generate(client, EDIT, 'edit', 42, Path(directory)/'out.png', reference_image=scene)

    def test_two_inputs_uploaded_separately_and_bound_to_correct_nodes(self):
        with tempfile.TemporaryDirectory() as directory:
            primary=Path(directory)/'scene.png'; primary.write_bytes(png())
            reference=Path(directory)/'reference.png'; reference.write_bytes(png(16, 8))
            profile={**EDIT, 'width':8, 'height':8, 'input_fit':'none'}
            client=MagicMock(); client.upload_image.side_effect=['uploaded-scene.png','uploaded-reference.png']; identity={}
            def request(route, payload=None, **kwargs):
                if route.startswith('/object_info/'):
                    return {'TextEncodeQwenImageEditPlus':{'input':{'optional':{'image2':['IMAGE']}}}}
                if route == '/prompt':
                    graph=payload['prompt']; identity['prefix']=graph['10']['inputs']['filename_prefix'].split('/')[-1]
                    self.assertEqual(graph['11']['inputs']['image'], 'uploaded-scene.png')
                    self.assertEqual(graph['16']['inputs']['image'], 'uploaded-reference.png')
                    return {'prompt_id':'owned'}
                if route.startswith('/history/'):
                    return {'owned':{'status':{'completed':True},'outputs':{'10':{'images':[{'filename':identity['prefix']+'.png','subfolder':'opengpu','type':'output'}]}}}}
                if route.startswith('/view?'): return png()
                raise AssertionError(route)
            client.request.side_effect=request
            with contextlib.redirect_stdout(io.StringIO()):
                result=media.generate(client,profile,'Use the reference palette',42,Path(directory)/'out.png',input_image=primary,reference_image=reference)
            self.assertEqual([c.args[0] for c in client.upload_image.call_args_list],[png(),png(16,8)])
            self.assertEqual(result['input_image_count'],2)
            self.assertEqual(primary.read_bytes(),png())
            self.assertEqual(reference.read_bytes(),png(16,8))

    def test_old_renderer_fails_before_upload_or_generation(self):
        with tempfile.TemporaryDirectory() as directory:
            source=Path(directory)/'scene.png'; source.write_bytes(png())
            client=MagicMock(); client.request.return_value={}
            with self.assertRaisesRegex(media.MediaError,'second reference'):
                media.generate(client,{**EDIT,'input_fit':'none'},'edit',42,Path(directory)/'out.png',input_image=source,reference_image=source)
            client.upload_image.assert_not_called()
            self.assertFalse(any(c.args[0]=='/prompt' for c in client.request.call_args_list))

    def test_custom_reference_record_does_not_replace_network_certificate(self):
        with tempfile.TemporaryDirectory() as directory:
            source=Path(directory)/'scene.png'; source.write_bytes(png())
            args=options(reference_image=source,input_image=source);args.action='plan'
            custom=media.edit_options(EDIT,args)
            self.assertNotEqual(media.profile_record(Path(directory),custom,'verified'),media.profile_record(Path(directory),EDIT,'verified'))
            args.action='setup'
            with self.assertRaises(media.MediaError):media.edit_options(EDIT,args)


if __name__ == '__main__':unittest.main()
