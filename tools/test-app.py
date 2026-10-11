#!/usr/bin/env python3
"""Check debug command correlation and device-shell argument quoting."""
import concurrent.futures
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

APP = Path(__file__).with_name('app.sh').resolve()
ADB = r'''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shlex
import sys
root = Path(os.environ['APP_TEST_DIR'])
if sys.argv[1] == 'shell':
    args = shlex.split(sys.argv[2])
    extras = {}
    for i, value in enumerate(args):
        if value == '--es':
            extras[args[i + 1]] = args[i + 2]
    request = extras.get('request', 'legacy')
    (root / (request + '.reply')).write_text('I/noritest: ' + request + ' ' + json.dumps(extras) + '\n')
else:
    for path in root.glob('*.reply'):
        print(path.read_text(), end='')
    print('I/noritest: unrelated reply')
'''

class AppCommands(unittest.TestCase):
    def test_concurrent_replies_preserve_arguments(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            adb = root / 'adb'
            adb.write_text(ADB)
            adb.chmod(0o755)
            env = dict(os.environ, APP_TEST_DIR=directory,
                       PATH=directory + os.pathsep + os.environ['PATH'])
            arguments = [('state',), ('remote', 'view'),
                         ('play', "search:owner's song | $(false) `false`"),
                         ('set', 'name', "a'b \" $HOME")]
            def run(args):
                result = subprocess.run([str(APP), *args], env=env, text=True,
                                        capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                reply = json.loads(result.stdout)
                self.assertEqual([reply[key] for key in ('cmd', 'arg', 'value')[:len(args)]], list(args))
                return reply['request']
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                requests = list(pool.map(run, arguments))
            self.assertEqual(len(set(requests)), len(arguments))

if __name__ == '__main__':
    unittest.main()
