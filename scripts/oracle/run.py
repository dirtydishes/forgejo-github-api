"""Run the fixed read-only gh cases on either host. Never write the environment."""
import json
import os
import re
from pathlib import Path
import subprocess
import sys

from scrub import emails

ROOT = Path(__file__).resolve().parent
CASES = json.loads((ROOT / 'cases.json').read_text())
REPO = 'dirtydishes/gfj-oracle'


def allowed(args):
    return args == ['pr', 'status'] or (
        len(args) in (3, 5)
        and args[:2] in (['pr', 'view'], ['pr', 'checks'])
        and args[2] in ('1', '2', '4', '5', '6', '7')
        and (len(args) == 3 or (args[1] == 'view' and args[3] == '--json'
             and all(field.isidentifier() for field in args[4].split(','))))
    )


def run(host, destination):
    destination.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(GH_HOST=host, GH_PROMPT_DISABLED='1', GH_PAGER='cat', NO_COLOR='1', CLICOLOR='0', TZ='UTC')
    env.pop('GH_DEBUG', None)
    for case in CASES:
        args = case['args']
        # Fixed verbs, fixed numeric selectors, and one exact status command. No shell.
        if not allowed(args) or re.fullmatch(r'[a-z0-9-]+', case['id']) is None:
            raise SystemExit('command outside read-only allowlist')
        result = subprocess.run(['gh', *args, '-R', f'{host}/{REPO}'], env=env, capture_output=True)
        prefix = destination / case['id']
        prefix.with_suffix('.stdout').write_bytes(emails(result.stdout))
        prefix.with_suffix('.stderr').write_bytes(emails(result.stderr))
        prefix.with_suffix('.exit').write_text(str(result.returncode) + '\n')
        print(f"captured {case['id']}: exit {result.returncode}")
        if host == 'github.com' and result.returncode != case['exit']:
            raise SystemExit(f"oracle {case['id']} exited {result.returncode}, expected {case['exit']}")


if __name__ == '__main__':
    host, output = sys.argv[1:]
    run(host, Path(output))
