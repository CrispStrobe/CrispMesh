import json
from pathlib import Path
import tempfile
import unittest
from scripts.photo_regression.stage_cache import StageCache, inventory

class RestartReceipts(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.output=self.root/'stage';self.output.mkdir();(self.output/'data').write_bytes(b'own output')
    def tearDown(self):self.temp.cleanup()
    def test_missing_and_changed_outputs_cannot_hit_cache(self):
        cache=StageCache(self.root,'same-input')
        self.assertFalse(cache.hit('phase',['tool'],['stage']))
        cache.record('phase',['tool'],['stage'])
        resumed=StageCache(self.root,'same-input',resume=True)
        self.assertTrue(resumed.hit('phase',['tool'],['stage']))
        self.assertFalse(resumed.hit('phase',['other-tool'],['stage']))
        (self.output/'data').write_bytes(b'changed')
        self.assertFalse(resumed.hit('phase',['tool'],['stage']))
    def test_outputs_cannot_escape_the_run(self):
        for path in ['../other','/outside']:
            with self.assertRaisesRegex(ValueError,'inside'):inventory(self.root,[path])
    def test_changed_recipe_is_refused(self):
        StageCache(self.root,'first')
        with self.assertRaisesRegex(ValueError,'changed'):StageCache(self.root,'second',resume=True)
    def test_success_without_outputs_is_not_recorded(self):
        cache=StageCache(self.root,'same')
        with self.assertRaisesRegex(ValueError,'without'):cache.record('phase',['tool'],['absent'])
        self.assertNotIn('phase',cache.entries)
    def test_predecessor_migration_requires_explicit_operator_flag_and_successful_stage(self):
        (self.root/'timings.json').write_text(json.dumps([{'stage':'complete','seconds':1}]))
        with self.assertRaisesRegex(ValueError,'operator adoption'):StageCache(self.root,'same',resume=True)
        cache=StageCache(self.root,'same',resume=True,adopt_completed=True)
        self.assertFalse(cache.hit('interrupted',['tool'],['stage']))
        self.assertTrue(cache.hit('complete',['tool'],['stage']))
        self.assertTrue(cache.entries['complete']['adopted_local_predecessor'])
