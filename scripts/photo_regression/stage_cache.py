"""Content-checked restart receipts for trusted local capture stages.

A cache hit requires the same input/recipe fingerprint, command and every output
byte. No source poses/masks are invented and incomplete stages never count.
"""
import hashlib
import json
from pathlib import Path


def sha(path):
    result=hashlib.sha256()
    with Path(path).open('rb') as stream:
        while block:=stream.read(1<<20):result.update(block)
    return result.hexdigest()


def inventory(root, paths):
    root=Path(root);result={}
    for relative in paths:
        if Path(relative).is_absolute() or '..' in Path(relative).parts:raise ValueError('stage output must stay inside its run')
        base=root/relative
        if not base.exists():return None
        files=sorted(p for p in base.rglob('*') if p.is_file()) if base.is_dir() else [base]
        if not files:return None
        for path in files:result[str(path.relative_to(root))]=sha(path)
    return result


class StageCache:
    def __init__(self,root,fingerprint,resume=False,adopt_completed=False):
        self.root=Path(root);self.path=self.root/'capture-cache.json';self.entries={};self.adopt=set()
        if self.path.exists():
            data=json.loads(self.path.read_text())
            if data['fingerprint']!=fingerprint:raise ValueError('capture inputs, tools or recipe changed; start a fresh run')
            if not resume:raise ValueError('output exists; use --resume for the same capture')
            self.entries=data['stages']
        elif resume:
            if not adopt_completed:raise ValueError('older output has no restart receipts; operator adoption is required')
            self.adopt={row['stage'] for row in json.loads((self.root/'timings.json').read_text())}
        self.fingerprint=fingerprint;self.save()
    def command_key(self,command):
        # Same paths under this run, with no shell interpretation.
        return hashlib.sha256(json.dumps(list(map(str,command)),separators=(',',':')).encode()).hexdigest()
    def hit(self,name,command,paths):
        if not paths:return False
        current=inventory(self.root,paths)
        if current is None:return False
        key=self.command_key(command)
        if name in self.adopt:
            self.entries[name]={'command':key,'outputs':current,'adopted_local_predecessor':True};self.adopt.remove(name);self.save();return True
        record=self.entries.get(name)
        return record is not None and record['command']==key and record['outputs']==current
    def record(self,name,command,paths):
        if not paths:return
        outputs=inventory(self.root,paths)
        if outputs is None:raise ValueError('stage returned success without its declared outputs: '+name)
        self.entries[name]={'command':self.command_key(command),'outputs':outputs};self.save()
    def save(self):
        pending=self.path.with_suffix('.pending')
        pending.write_text(json.dumps({'fingerprint':self.fingerprint,'stages':self.entries},indent=2)+'\n');pending.replace(self.path)
