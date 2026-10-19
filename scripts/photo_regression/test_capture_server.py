import json
from pathlib import Path
import tempfile
import unittest
from collections import namedtuple
from unittest.mock import patch
from scripts.turntable_mesh.engine_server import Engine, ThreadingHTTPServer, handler_for
from scripts.photo_regression.stage_cache import inventory
import threading
import urllib.request
import urllib.error

class CaptureServer(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        self.data=self.root/'data';self.data.mkdir();(self.data/'photos').mkdir()
        self.tools=self.root/'tools.json';self.tools.write_text(json.dumps({'native':'/host/native'}))
        self.engine=Engine(self.root/'runs',self.data,capture_tools=self.tools)
        self.disk=patch('scripts.turntable_mesh.engine_server.shutil.disk_usage',return_value=namedtuple('Disk','total used free')(100<<30,10<<30,90<<30));self.disk.start()
    def tearDown(self):self.disk.stop();self.tmp.cleanup()
    def fake_child(self,command,**kwargs):
        self.command=command;output=Path(command[command.index('--output')+1]);output.mkdir()
        (output/'events.jsonl').write_text('{}\n')
        class Child:
            def poll(self):return None
        return Child()
    def test_request_has_fixed_defaults_and_opt_in_gaussian_and_preserves_stereo(self):
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'photos':'photos','calibration':'auto'})
        self.assertIn('--base-surface',self.command)
        self.assertEqual(self.command[self.command.index('--base-surface')+1],'silhouette')
        self.assertNotIn('--gaussian',self.command)
        self.assertIn('--no-refine-masks',self.command)
        self.engine.capture_child=None
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'photos':'photos','name':'gaussian','base_surface':'stereo','splats':{}})
        # Empty object is also an explicit request for default Gaussian refinement.
        self.assertIn('stereo',self.command)
        self.assertIn('--gaussian',self.command)
        self.assertEqual(self.command[self.command.index('--gaussian-quality')+1],'capture')
    def test_model_paths_cannot_be_injected_as_numeric_settings(self):
        for item in ['init=/host/file','crop=1;echo unsafe','steps=NaN']:
            with self.assertRaisesRegex(ValueError,'override'):
                self.engine.start_run({'photos':'photos','splats':{'quality':'capture','settings':[item]}})
    def test_source_escape_and_unrecognized_modes_are_refused(self):
        for body in [{'photos':'../outside'},{'photos':'photos','base_surface':'unknown'}]:
            with self.assertRaises(ValueError):self.engine.start_run(body)
    def test_known_active_capture_prevents_overlapping_heavy_runs(self):
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'photos':'photos'})
        with self.assertRaisesRegex(ValueError,'already running'):self.engine.start_run({'photos':'photos'})

    def test_app_preview_masks_crop_padding_and_height_groups_reach_the_pipeline(self):
        masks=self.data/'capture-masks';masks.mkdir()
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'photos':'photos','providers':{'masks':'import'},'masks_import':masks.name,'photo_options':['--object-crop-padding','0.3','--turntable-turn-sizes','36,35']})
        self.assertEqual(self.command[self.command.index('--masks')+1],str(masks.resolve()))
        self.assertEqual(self.command[self.command.index('--object-crop-padding')+1],'0.3')
        self.assertEqual(self.command[self.command.index('--capture-groups')+1],'36,35')

    def test_mask_refinement_is_optional(self):
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'photos':'photos','providers':{'masks':'automatic'}})
        self.assertNotIn('--refine-masks',self.command)
        self.engine.capture_child=None
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'photos':'photos','name':'refined','providers':{'masks':'refined'}})
        self.assertIn('--refine-masks',self.command)

    def test_installed_refinement_is_the_default_and_native_masks_remain_available(self):
        source=self.root/'sam-source';source.mkdir();checkpoint=self.root/'sam.pt';checkpoint.write_bytes(b'fixture')
        self.tools.write_text(json.dumps({'native':'/host/native','sam_source':str(source),'sam_checkpoint':str(checkpoint)}))
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'photos':'photos'})
        self.assertIn('--refine-masks',self.command)
        self.assertNotIn('--no-refine-masks',self.command)
        self.engine.capture_child=None
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'photos':'photos','name':'native','providers':{'masks':'automatic'}})
        self.assertIn('--no-refine-masks',self.command)

    def test_capture_options_cannot_inject_other_program_or_data_flags(self):
        for flags in [['--masks','/outside'],['--native','/program'],['--object-crop-padding','NaN'],['--turntable-turn-sizes','36;unsafe']]:
            with self.assertRaises(ValueError):self.engine.start_run({'photos':'photos','photo_options':flags})

    def test_refinement_reuses_only_unchanged_recovered_inputs(self):
        scene=self.data/'recovered-test';scene.mkdir();(scene/'cameras.json').write_text('{}')
        run=self.engine.runs/'parent';run.mkdir()
        (run/'events.jsonl').write_text('{"type":"run_finished","status":"complete"}\n')
        (run/'recovered-scene.json').write_text(json.dumps({'inputs':scene.name,'sha256':inventory(scene,['.'])}))
        with patch.object(self.engine,'start_run',return_value={'id':'refined'}) as start:
            self.assertEqual(self.engine.refine_run('parent'),{'id':'refined'})
            self.assertEqual(start.call_args.args[0]['splats'],{'quality':'capture'})
            self.assertEqual(start.call_args.args[0]['inputs'],scene.name)
            (run/'pipeline.json').write_text(json.dumps({'base_surface':'stereo'}))
            self.engine.refine_run('parent')
            self.assertEqual(start.call_args.args[0]['base_surface'],'stereo')
        (scene/'cameras.json').write_text('{"changed":true}')
        with self.assertRaisesRegex(ValueError,'changed'):self.engine.refine_run('parent')

    def test_prepared_input_runs_also_record_content_before_later_refinement(self):
        scene=self.data/'prepared';scene.mkdir();(scene/'cameras.json').write_text('{}')
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            run=self.engine.start_run({'inputs':scene.name})
        record=json.loads((self.engine.runs/(run['id']+'.recovered-scene.json')).read_text())
        self.assertEqual(record['sha256'],inventory(scene,['.']))
        (scene/'cameras.json').write_text('{"changed":true}')
        with self.assertRaisesRegex(ValueError,'changed'):self.engine.refine_run(run['id'])

    def test_cached_refinement_uses_full_capture_checks_and_keeps_dense_overrides(self):
        scene=self.data/'prepared';scene.mkdir();(scene/'cameras.json').write_text('{}')
        validation=self.data/'validation';validation.mkdir();(validation/'cameras.json').write_text('{}')
        with patch('scripts.turntable_mesh.engine_server.subprocess.Popen',side_effect=self.fake_child):
            self.engine.start_run({'inputs':scene.name,'validation_inputs':validation.name,
                'splats':{'quality':'capture','settings':['steps=2']},'settings':{'grid':192}})
        self.assertIn('scripts.photo_regression.prepared_capture',self.command)
        self.assertIn('--gaussian',self.command)
        self.assertEqual(self.command[self.command.index('--validation-inputs')+1],str(validation.resolve()))
        self.assertIn('--set',self.command)
        self.assertIn('grid=192',self.command)

    def test_cached_validation_views_are_content_checked_too(self):
        scene=self.data/'prepared';scene.mkdir();(scene/'cameras.json').write_text('{}')
        validation=self.data/'validation';validation.mkdir();(validation/'cameras.json').write_text('{}')
        run=self.engine.runs/'parent';run.mkdir()
        (run/'events.jsonl').write_text('{"type":"run_finished","status":"complete"}\n')
        (run/'recovered-scene.json').write_text(json.dumps({'inputs':scene.name,'sha256':inventory(scene,['.']),
            'validation_inputs':validation.name,'validation_sha256':inventory(validation,['.'])}))
        with patch.object(self.engine,'start_run',return_value={'id':'refined'}) as start:
            self.engine.refine_run('parent')
            self.assertEqual(start.call_args.args[0]['validation_inputs'],validation.name)
        (validation/'cameras.json').write_text('changed')
        with self.assertRaisesRegex(ValueError,'changed'):self.engine.refine_run('parent')

    def test_http_upload_is_atomic_and_cannot_write_run_metadata(self):
        server=ThreadingHTTPServer(('127.0.0.1',0),handler_for(self.engine,None,None))
        worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
        try:
            base=f'http://127.0.0.1:{server.server_port}/api/uploads'
            def upload(folder,name):
                return urllib.request.urlopen(urllib.request.Request(base+f'?folder={folder}&name={name}',data=b'photo bytes',method='POST'))
            with upload('capture-test','photo.jpg') as response:
                self.assertEqual(response.status,201)
                self.assertEqual(json.load(response)['path'],'capture-test/photo.jpg')
            self.assertEqual((self.data/'capture-test/photo.jpg').read_bytes(),b'photo bytes')
            for folder,name,status in [('capture-test','photo.jpg',409),('recovered-test','cameras.json',400),('capture-test','../escape.jpg',400)]:
                with self.assertRaises(urllib.error.HTTPError) as failure:upload(folder,name)
                self.assertEqual(failure.exception.code,status)
            pending=self.data/'capture-test/second.jpg.pending';pending.write_bytes(b'other uploader')
            with self.assertRaises(urllib.error.HTTPError):upload('capture-test','second.jpg')
            self.assertEqual(pending.read_bytes(),b'other uploader')
        finally:
            server.shutdown();server.server_close();worker.join()
