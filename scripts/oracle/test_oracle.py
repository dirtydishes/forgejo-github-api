"""Check the capture allowlist and ensure expected failures cannot hide new differences."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import sys
sys.dont_write_bytecode = True
import unittest
from unittest.mock import patch
import subprocess

import compare
import run
import scrub


class OracleTests(unittest.TestCase):
    def test_email_scrubbing_at_capture_and_comparison(self):
        raw = b'Author <first.last+tag@sub.example.com>, OTHER@EXAMPLE.ORG; "email":"dirtydishes@users.noreply.invalid"\n'
        expected = b'Author <email@example.invalid>, email@example.invalid; "email":"email@example.invalid"\n'
        self.assertEqual(scrub.emails(raw), expected)
        self.assertEqual(scrub.emails(expected), expected)
        self.assertEqual(compare.normalize(raw.decode(), 'wide-1'), expected.decode())
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp)
            result = subprocess.CompletedProcess([], 0, raw, raw)
            with patch.object(run, 'CASES', [dict(id='view-1', args=['pr', 'view', '1'], exit=0)]), \
                    patch.object(run.subprocess, 'run', return_value=result), \
                    contextlib.redirect_stdout(io.StringIO()):
                run.run('github.com', output)
            self.assertEqual((output / 'view-1.stdout').read_bytes(), expected)
            self.assertEqual((output / 'view-1.stderr').read_bytes(), expected)

    def test_allowlist_rejects_writes_extra_flags_and_shell_text(self):
        for args in [[], ['pr'], ['pr', 'merge', '1'], ['pr', 'view', '8'],
                     ['api', 'repos/dirtydishes/gfj-oracle'], ['pr', 'checks', '1', '--watch'],
                     ['pr', 'view', '1; touch hacked'], ['pr', 'view', '1', '--web'],
                     ['pr', 'view', '1', '--json', 'title; echo secret'],
                     ['pr', 'status', '--something']]:
            self.assertFalse(run.allowed(args), args)
        for case in run.CASES:
            self.assertTrue(run.allowed(case['args']), case['id'])

    def test_debug_capture_keeps_documents_without_headers(self):
        debug = '> Authorization: token secret\nGraphQL query:\nquery Example { viewer { login } }\nGraphQL variables: {"q":"author:@me"}\n\n< Response\n{\n  "query":"query Introspection { __type(name: \"Issue\") { name } }"\n}\n> GET /repos/o/r/readme HTTP/1.1\n'
        requests = run.debug_requests(debug)
        self.assertEqual(requests[0]['variables'], {'q': 'author:@me'})
        self.assertEqual(requests[-1], {'method': 'GET', 'path': '/repos/o/r/readme'})
        self.assertNotIn('secret', json.dumps(requests))

    def test_new_expected_failures_accept_only_rank_or_absent_run_data(self):
        a = '[{"number":1},{"number":2}]'
        b = '[{"number":2},{"number":1}]'
        self.assertTrue(compare.expected_difference('list-search', a, b))
        for wrong in ('[]', '[{"number":2}]', '[{"number":2},{"number":2}]', '[{"number":2,"title":"changed"},{"number":1}]'):
            self.assertFalse(compare.expected_difference('list-search', a, wrong))
        oracle = (Path(__file__).resolve().parents[2] / 'fixtures/oracle/runs.stdout').read_text()
        self.assertTrue(compare.expected_difference('runs', oracle, ''))
        self.assertFalse(compare.expected_difference('runs', oracle, 'unexpected run\n'))
        self.assertFalse(compare.expected_difference('runs', '', ''))

    def test_normalization_preserves_semantics(self):
        a = compare.normalize('{"state":"OPEN","number":1,"detailsUrl":"https://github.com/check","id":"GITHUB_ID","url":"https://github.com/o/r/pull/1","createdAt":"2026-10-30T19:00:00Z"}', 'probe-1')
        b = compare.normalize('{"state":"OPEN","number":1,"detailsUrl":"https://forge.test/check","id":"FGA_ID","url":"https://forge.test/o/r/pulls/1","createdAt":"2020-01-01T00:00:00Z"}', 'probe-1')
        self.assertEqual(a, b)
        self.assertNotEqual(a, b.replace('OPEN', 'CLOSED'))
        self.assertEqual(compare.normalize('check\tpass\t5s\thttps://github.com/check\t\n', 'checks-1'),
                         compare.normalize('check\tpass\t0s\thttps://forge.test/check\t\n', 'checks-1'))

    def test_expected_failure_rejects_unrelated_differences_and_fixed_cases(self):
        root = Path(__file__).resolve().parents[2] / '.local'
        root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=root) as temp:
            expected, actual = Path(temp) / 'expected', Path(temp) / 'actual'
            expected.mkdir(); actual.mkdir()
            for case in run.CASES:
                for ext in ('stdout', 'stderr', 'exit'):
                    output = '{"title":"scenario: draft","mergeStateStatus":"CLEAN","mergeable":"MERGEABLE"}\n' if ext == 'stdout' else '0\n' if ext == 'exit' else ''
                    (expected / f"{case['id']}.{ext}").write_text(output)
                    if ext == 'stdout' and case['id'] == 'probe-4':
                        output = output.replace('CLEAN', 'UNKNOWN').replace('MERGEABLE', 'UNKNOWN')
                    (actual / f"{case['id']}.{ext}").write_text(output)
            def check():
                with contextlib.redirect_stdout(io.StringIO()):
                    with patch.object(compare, 'EXPECTED_FAILURES', {'probe-4': compare.EXPECTED_FAILURES['probe-4']}):
                        return compare.main(expected, actual)
            self.assertEqual(check(), 0)
            draft = actual / 'probe-4.stdout'
            original = draft.read_text()
            draft.write_text(original.replace('scenario: draft', 'WIP: scenario: draft'))
            self.assertEqual(check(), 1, 'draft title differences must fail now')
            draft.write_text(original)
            for case in ('view-4', 'status'):
                path = actual / f'{case}.stdout'
                original = path.read_text()
                path.write_text(original.replace('scenario: draft', 'WIP: scenario: draft'))
                self.assertEqual(check(), 1, f'{case} no longer allows title differences')
                path.write_text(original)
            (actual / 'probe-4.exit').write_text('1\n')
            self.assertEqual(check(), 1, 'a new error cannot pass as an expected draft difference')
            (actual / 'probe-4.exit').write_text('0\n')
            (actual / 'probe-4.stdout').write_text((expected / 'probe-4.stdout').read_text())
            self.assertEqual(check(), 1, 'a fixed case must force removal of its xfail')
            (actual / 'probe-4.stdout').write_text('')
            self.assertEqual(check(), 1, 'missing JSON must produce an unexpected result, not crash')
            report = json.loads((actual / 'report.json').read_text())
            self.assertEqual(next(c for c in report if c['id'] == 'probe-4')['result'], 'unexpected')


if __name__ == '__main__':
    unittest.main()
