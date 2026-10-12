"""Reading storage boundaries are byte limits; source snapshots stay complete."""
import argparse
import copy
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from html import escape
import freshloop_agent as agent
import review_comparison as review
from test_freshloop_agent import artifact


class ReadingStorageLimitTests(unittest.TestCase):
    def test_source_boundaries_count_utf8_bytes(self):
        exact = '中' * (agent.MAX_READING_SOURCE_BYTES // 3) + 'aa'
        self.assertEqual(len(exact.encode()), agent.MAX_READING_SOURCE_BYTES)
        for field in ('reader_markdown', 'plain_text'):
            with self.subTest(field=field):
                data = artifact(); data[field] = exact
                self.assertEqual(agent.validate(data, 'reading_article'), [])
                data[field] += 'x'
                errors = agent.validate(data, 'reading_article')
                self.assertTrue(any(field in e and '2000000 bytes' in e and 'actual: 2000001 bytes' in e for e in errors))

    def test_compressed_boundaries_do_not_inherit_source_budget(self):
        data = artifact(); data['compressed_markdown'] = 'x' * agent.MAX_READING_MANUSCRIPT_BYTES
        self.assertEqual(agent.validate(data, 'reading_article'), [])
        data['compressed_markdown'] += 'x'
        self.assertTrue(any('compressed_markdown exceeds 80000 bytes' in e for e in agent.validate(data, 'reading_article')))
        data = artifact(); data['audio_script'] = '中' * 6501
        self.assertTrue(any('6500 characters' in e for e in agent.validate(data, 'reading_article')))

    def test_complete_long_article_is_valid_and_never_mutated(self):
        path = os.environ.get('FRESHLOOP_READING_TEST_ARTIFACT')
        data = agent.read(path) if path else artifact()
        if not path:
            data['reader_markdown'] = ('An article explains its experiment and constraints.\n' * 2200) + 'FINAL IMPORTANT LIMITATION.'
        data['editorial']['ready'] = True  # isolate storage validation in this fixture
        original = copy.deepcopy(data)
        self.assertGreater(len(data['reader_markdown'].encode()), 80000)
        self.assertEqual(agent.validate(data, 'reading_article'), [])
        self.assertEqual(data, original)

    def test_actual_oversized_article_error_is_recorded_before_transport(self):
        data = artifact()
        paragraph = 'The experiment states conditions, repeats observations and preserves uncertainty.\n'
        data['reader_markdown'] = paragraph * (agent.MAX_READING_SOURCE_BYTES // len(paragraph.encode()) + 1)
        actual = len(data['reader_markdown'].encode())
        self.assertGreater(actual, agent.MAX_READING_SOURCE_BYTES)
        with tempfile.TemporaryDirectory() as directory:
            draft = Path(directory) / 'reading_article.json'; agent.save(draft, data)
            args = argparse.Namespace(command='validate', job_type='reading_article', artifact=str(draft), state=None, ledger=None)
            with patch.object(agent.Api, 'call', side_effect=AssertionError('validation must not contact a service')):
                self.assertTrue(agent.dispatch(args))
            issues = agent.read(Path(directory) / 'editorial-issues.json')
            encoded = __import__('json').dumps(issues)
            self.assertIn('reader_markdown exceeds 2000000 bytes', encoded)
            self.assertIn('actual: ' + str(actual) + ' bytes', encoded)
            self.assertEqual(agent.read(draft)['reader_markdown'], data['reader_markdown'])

    def test_full_snapshot_contains_late_constraints_and_hashes_tail_changes(self):
        data = artifact()
        body = ('A complete experiment paragraph gives evidence and conditions.\n' * 1700) + 'FINAL LIMITATION: cache results are not physical disk throughput.'
        data['reader_markdown'] = body
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); draft = root / 'reading_article.json'; source = root / 'source.json'
            agent.save(draft, data)
            raw = dict(id='long-source', entry=dict(link=data['original_url']), original_html='<article><pre>'+escape(body)+'</pre></article>', fetch_error=None)
            agent.save(source, raw)
            meta = review.prepare_inputs(draft, [str(source)], root / 'inputs.json')
            packet = agent.read(root / 'inputs.json')
            snapshot = agent.read(packet['sources'][0]['snapshot'])
            self.assertTrue(snapshot['body'].endswith('FINAL LIMITATION: cache results are not physical disk throughput.'))
            self.assertGreater(len(snapshot['body'].encode()), 80000)
            original_hash = packet['sources'][0]['body_sha256']
            raw['original_html'] += '<!-- metadata only -->'
            agent.save(source, raw)
            self.assertNotEqual(review.source_input(source)['source_sha256'], packet['sources'][0]['source_sha256'])
            raw['original_html'] = '<article><pre>'+escape(body+' Changed final constraint.')+'</pre></article>'
            agent.save(source, raw)
            self.assertNotEqual(review.source_input(source)['body_sha256'], original_hash)
            self.assertEqual(meta['artifact_sha256'], review.digest(data))


if __name__ == '__main__':
    unittest.main()
