import argparse
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

from test_runtime import media, png

EDIT = json.loads(Path(__file__).with_name('qwen-image-edit-v1.json').read_text())


def options(**kw):
    values = dict(quality=False, preserve_size=False, width=None, height=None, steps=None, input_image=None, fast=False)
    values.update(kw)
    return argparse.Namespace(**values)


class EditQualityTests(unittest.TestCase):
    def test_original_size_pads_without_scaling_and_crops_only_padding(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'input.png'; source.write_bytes(png(65, 97))
            profile = media.edit_options(EDIT, options(preserve_size=True, input_image=source, quality=True))
        self.assertEqual((profile['width'], profile['height']), (128, 128))
        self.assertEqual((profile['output_width'], profile['output_height']), (65, 97))
        self.assertEqual(profile['input_fit'], 'pad_original')
        graph = media.workflow(profile, 'change background', 42, 'owned', 'input.png')
        self.assertEqual(graph['15']['inputs'], dict(image=['9', 0], width=65, height=97, x=0, y=0))
        self.assertEqual(graph['10']['inputs']['images'], ['15', 0])
        self.assertEqual(EDIT['width'], 832)

    def test_aligned_input_is_not_reencoded_or_resized(self):
        source = png(64, 128)
        profile = {**EDIT, 'width':64, 'height':128, 'input_fit':'pad_original'}
        with patch.object(media.subprocess, 'run') as process:
            self.assertIs(media.fit_input_image(source, profile), source)
            process.assert_not_called()

    def test_standard_sampling_removes_lightning_instead_of_adding_steps_to_it(self):
        profile = media.edit_options(EDIT, options(quality=True, steps=40))
        graph = media.workflow(profile, 'change background', 42, 'owned')
        self.assertNotIn('13', graph)
        self.assertEqual(graph['7']['inputs']['model'], ['1', 0])
        self.assertEqual(graph['8']['inputs']['steps'], 40)
        self.assertEqual(graph['8']['inputs']['cfg'], 4.0)
        self.assertNotEqual(media.profile_record(Path('.'), EDIT, 'verified'), media.profile_record(Path('.'), profile, 'verified'))
        self.assertIn('13', media.workflow(EDIT, 'edit', 1, 'owned'))

    def test_explicit_canvas_and_invalid_combinations(self):
        profile = media.edit_options(EDIT, options(width=1024, height=1536, quality=True))
        self.assertEqual((profile['width'], profile['height'], profile['steps']), (1024, 1536, 30))
        for kw in [dict(width=1024), dict(width=1000, height=1024), dict(width=2048, height=2048), dict(steps=30), dict(quality=True, steps=4), dict(quality=True, fast=True), dict(preserve_size=True), dict(preserve_size=True, width=1024, height=1024)]:
            with self.subTest(kw=kw), self.assertRaises(media.MediaError): media.edit_options(EDIT, options(**kw))
        with self.assertRaises(media.MediaError): media.edit_options({**EDIT, 'operation':'text_to_video'}, options(quality=True))

    def test_oversized_original_rejected_without_silent_downscale(self):
        with tempfile.TemporaryDirectory() as directory:
            source=Path(directory)/'large.png'; source.write_bytes(png(2048, 2048))
            with self.assertRaisesRegex(media.MediaError, 'No silent downscaling'):
                media.edit_options(EDIT, options(preserve_size=True, input_image=source))

    def test_generation_validates_exact_original_output_dimensions(self):
        with tempfile.TemporaryDirectory() as directory:
            source=Path(directory)/'input.png'; source.write_bytes(png(65, 97))
            profile=media.edit_options(EDIT, options(preserve_size=True, input_image=source, quality=True))
            client=MagicMock(); identity={}
            def request(route, payload=None, **kwargs):
                if route == '/prompt':
                    identity['prefix']=payload['prompt']['10']['inputs']['filename_prefix'].split('/')[-1]
                    return dict(prompt_id='owned')
                if route.startswith('/history/'):
                    return {'owned':{'status':{'completed':True},'outputs':{'10':{'images':[{'filename':identity['prefix']+'.png','subfolder':'opengpu','type':'output'}]}}}}
                if route.startswith('/view?'): return png(65, 97)
                raise AssertionError(route)
            client.request.side_effect=request
            with patch.object(media, 'fit_input_image', return_value=png(128, 128)):
                output=Path(directory)/'edited.png'
                media.generate(client, profile, 'change background', 42, output, input_image=source)
            media.validate_png(output.read_bytes(), 65, 97)


if __name__ == '__main__': unittest.main()
