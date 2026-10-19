import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from scripts.photo_regression.capture_pipeline import select_capture_surface
from scripts.turntable_mesh.dense_events import EventLog

class SurfaceSelection(unittest.TestCase):
    def test_rejected_gaussian_retains_base_and_both_texture_downloads(self):
        with tempfile.TemporaryDirectory() as folder:
            out=Path(folder);(out/'reconstruction/mesh').mkdir(parents=True)
            base=out/'reconstruction/mesh/mesh.stl';base.write_bytes(b'base geometry')
            (base.parent/'result.json').write_text(json.dumps({'triangles':143738,'closed':True}))
            old=out/'reconstruction/texture/mesh.glb';old.parent.mkdir();old.write_bytes(b'gaussian GLB')
            commands=[]
            def execute(name,command,stage=None):
                commands.append(list(map(str,command)))
                target=Path(command[command.index('--output')+1]);target.mkdir()
                if name=='base-texture':(target/'mesh.glb').write_bytes(b'base GLB')
                else:(target/'result.json').write_text(json.dumps({'silhouette_iou_input_masks':{'median':.799701 if name=='all-view-check' else .824438}}))
            args=SimpleNamespace(output=out,gaussian=True,base_surface='silhouette',texture=True)
            primary,selection=select_capture_surface(args,'native',Path('train'),Path('all-views'),68,{},execute,EventLog(out/'events.jsonl'))
            self.assertEqual(primary,base);self.assertFalse(selection['gaussian_selected'])
            self.assertEqual(json.loads((out/'surface-choice.json').read_text()),selection)
            records=[json.loads(row) for row in (out/'events.jsonl').read_text().splitlines()]
            meshes=[row for row in records if row.get('kind')=='final_mesh']
            self.assertEqual(meshes[-1]['path'],'reconstruction/mesh/mesh.stl')
            textures=[row for row in records if row.get('kind')=='textured_mesh']
            self.assertEqual([(row['stage'],row['path']) for row in textures],[('splats','reconstruction/texture/mesh.glb'),('texture','base-texture/mesh.glb')])
            self.assertEqual(old.read_bytes(),b'gaussian GLB')
            self.assertTrue(all(command[command.index('--inputs')+1]=='all-views' for command in commands[:2]))
            self.assertIn(str(base),commands[-1])

    def test_recovered_export_retains_full_validation_beyond_selected_views(self):
        from scripts.photo_regression.capture_pipeline import retain_recovered_scene
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);train=root/'train';full=root/'full';data=root/'data'
            train.mkdir();full.mkdir()
            (train/'cameras.json').write_text('{"views":[1]}');(full/'cameras.json').write_text('{"views":[1,2]}')
            (train/'image.png').write_bytes(b'own pixels');(full/'image.png').write_bytes(b'own pixels')
            (full/'excluded.png').write_bytes(b'excluded own view')
            record=retain_recovered_scene(train,full,data,'a'*64)
            validation=data/record['validation_inputs']
            self.assertNotEqual(record['inputs'],record['validation_inputs'])
            self.assertTrue((validation/'excluded.png').exists())
            self.assertNotEqual((validation/'cameras.json').stat().st_ino,(full/'cameras.json').stat().st_ino)
            (validation/'cameras.json').write_text('changed')
            with self.assertRaisesRegex(ValueError,'changed'):
                retain_recovered_scene(train,full,data,'a'*64)
            self.assertEqual((full/'cameras.json').read_text(),'{"views":[1,2]}')
