"""Compare real gh output with narrow normalizers and strict expected differences."""
import difflib
import json
from pathlib import Path
import re
import sys

from scrub import emails

CASES = json.loads((Path(__file__).parent / 'cases.json').read_text())


def text(value, case):
    value = emails(value.encode()).decode()
    value = value.replace('https://github.com/', 'https://HOST/').replace('https://forge.test/', 'https://HOST/')
    value = re.sub(r'/pulls/(\d+)', r'/pull/\1', value)
    value = re.sub(r'\b\d{4}-\d\d-\d\dT[^\s"\t]+', 'TIMESTAMP', value)
    value = re.sub(r'\b(?:about |less than )?\d+ (?:seconds?|minutes?|hours?|days?|weeks?|months?|years?) ago\b', 'RELATIVE_TIME', value)
    value = re.sub(r'\b[0-9a-f]{40}\b', 'SHA', value)
    if case.startswith('checks-'):
        # Tab columns are name, bucket, elapsed, link, event. Keep names, buckets, event.
        rows = []
        for line in value.splitlines():
            cols = line.split('\t')
            if len(cols) >= 4:
                cols[2] = 'DURATION'; cols[3] = 'CHECK_LINK'
            rows.append('\t'.join(cols))
        value = '\n'.join(rows) + ('\n' if value.endswith('\n') else '')
    return value


def normalize(value, case):
    try: data = json.loads(value)
    except ValueError: return text(value, case)
    def walk(obj, key=''):
        if isinstance(obj, dict): return {k: walk(v, k) for k,v in obj.items()}
        if isinstance(obj, list): return [walk(v, key) for v in obj]
        if isinstance(obj, str):
            if key in ('id', 'node_id'): return 'NODE_ID'
            if key in ('detailsUrl', 'targetUrl'): return 'CHECK_LINK'
            return text(obj, case)
        return obj
    return json.dumps(walk(data), sort_keys=True, indent=2) + '\n'


def known_difference(case, output):
    # Forgejo folds draft into mergeable=false. Only these two fields may differ.
    # Accept these exact values only, never arbitrary mismatches.
    if case in ('probe-4',):
        try: data = json.loads(output)
        except ValueError: return output
        if not isinstance(data, dict): return output
        if data.get('mergeStateStatus') == 'UNKNOWN': data['mergeStateStatus'] = 'CLEAN'
        if data.get('mergeable') == 'UNKNOWN': data['mergeable'] = 'MERGEABLE'
        return json.dumps(data, sort_keys=True, indent=2) + '\n'
    return output


def main(expected, actual):
    counts = dict(passed=0, xfail=0, unexpected=0)
    report = []
    for case in CASES:
        case = case['id']
        diffs = []
        matches = True
        known = True
        for ext in ('exit', 'stdout', 'stderr'):
            a = normalize((expected / f'{case}.{ext}').read_text(), case)
            b = normalize((actual / f'{case}.{ext}').read_text(), case)
            if a != b:
                matches = False
                known &= ext == 'stdout' and a == known_difference(case, b)
                diffs.extend(difflib.unified_diff(a.splitlines(), b.splitlines(), fromfile='oracle.'+ext, tofile='gateway.'+ext, lineterm=''))
        expected_failure = case == 'probe-4'
        state = 'pass' if matches and not expected_failure else 'xfail' if not matches and known and expected_failure else 'unexpected'
        counts[{'pass':'passed','xfail':'xfail','unexpected':'unexpected'}[state]] += 1
        reason = 'Forgejo draft mergeability is ambiguous: mergeable and mergeStateStatus are UNKNOWN instead of MERGEABLE/CLEAN' if state == 'xfail' else ''
        print(f'{state.upper()} {case}' + (': ' + reason if reason else ''))
        if state != 'pass': print('\n'.join(diffs) if diffs else 'expected difference disappeared; remove xfail after review')
        report.append(dict(id=case, result=state, reason=reason, diff=diffs))
    (actual / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f"Oracle summary: {counts['passed']} pass, {counts['xfail']} xfail, {counts['unexpected']} unexpected")
    return int(counts['unexpected'] != 0)


if __name__ == '__main__':
    sys.exit(main(*map(Path, sys.argv[1:])))
