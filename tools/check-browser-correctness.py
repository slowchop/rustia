#!/usr/bin/env python3
"""Check browser-runner rejection paths without launching a browser."""
from pathlib import Path
import os, shutil, subprocess, tempfile
repo=Path(__file__).resolve().parent.parent
root=repo
with tempfile.TemporaryDirectory(prefix='rustia-browser-review-') as tmp:
    sandbox=Path(tmp)
    (sandbox/'bin').mkdir(); (sandbox/'tools').mkdir()
    shutil.copy(repo/'tools/run-private-platform-golden.sh',sandbox/'tools/run-private-platform-golden.sh')
    script='''#!/usr/bin/env python3
import os,sys
name=os.path.basename(sys.argv[0]); args=sys.argv[1:]
if name=='uname': print(('Darwin' if os.getenv('STUB_SAFARI') else 'Linux') if args==['-s'] else ('arm64' if os.getenv('STUB_SAFARI') else 'x86_64'))
elif name=='firefox': print('Mozilla Firefox 123.0')
elif name=='geckodriver': print('geckodriver 0.36.0 (stub)')
elif name=='defaults': print('18.0')
elif name=='wasm-bindgen': print('wasm-bindgen 0.2.126')
elif name=='wasm-bindgen-test-runner': print('wasm-bindgen-test-runner '+('0.2.125' if os.getenv('STUB_WRONG_RUNNER') else '0.2.126'))
elif name=='rustc': print('rustc 1.95.0 (stub)')
elif name=='git': print('a'*40)
elif name=='cargo':
 if 'platform_golden' in args and not os.getenv('STUB_NO_MARKER'): print('RUSTIA_PRIVATE_PLATFORM_GOLDEN stub')
 print('test result: ok. '+('0' if os.getenv('STUB_ZERO_TESTS') else '1')+' passed; 0 failed; 0 ignored;')
'''
    for name in ['uname','firefox','geckodriver','defaults','wasm-bindgen','wasm-bindgen-test-runner','rustc','git','cargo']:
        f=sandbox/'bin'/name; f.write_text(script); f.chmod(0o755)
    env={**os.environ,'PATH':str(sandbox/'bin')+':'+os.environ['PATH']}
    # Test Firefox shell logic with fake tool output; no browser or Rust execution.
    def run(extra,version='123.0'):
        return subprocess.run(['bash',str(root/'tools/run-browser-correctness.sh'),'firefox',version,'0.36.0'],cwd=sandbox,env={**env,**extra},capture_output=True,text=True)
    cases=[('wrong-runner',{'STUB_WRONG_RUNNER':'1'},'123.0',False),('success',{},'123.0',True),('wrong-version',{},'1',False),('zero-tests',{'STUB_ZERO_TESTS':'1'},'123.0',False),('missing-golden',{'STUB_NO_MARKER':'1'},'123.0',False),('node-override',{'WASM_BINDGEN_USE_NODE_EXPERIMENTAL':''},'123.0',False)]
    for label,extra,version,success in cases:
        result=run(extra,version)
        assert (result.returncode==0)==success,(label,result.stdout,result.stderr)
        print(label+': passed')
