"""Behavior checks at the spike's acquisition and discussion boundaries."""
import tempfile
from pathlib import Path
import unittest
import harness


class AcquisitionTests(unittest.TestCase):
    def test_missing_fork_objects_preserve_dirty_checkout_on_all_outcomes(self):
        with tempfile.TemporaryDirectory() as directory:
            report = harness.local_trial(Path(directory))
        self.assertEqual(report['integrity'], {'success': True, 'failure': True, 'cancelled': True})
        self.assertTrue(report['missing_before'])
        self.assertNotEqual(report['base'], report['merge_base'])
        self.assertEqual(report['merge_base'], report['source_base'])
        self.assertTrue(report['deterministic'])
        anchors = report['anchors']
        self.assertIn({'old': 2, 'new': None, 'kind': 'delete'}, anchors['edit.txt']['lines'])
        self.assertIn({'old': None, 'new': 2, 'kind': 'add'}, anchors['edit.txt']['lines'])
        self.assertEqual(anchors['renamed.txt']['old_path'], 'rename.txt')
        self.assertEqual(anchors['binary.bin']['unsupported'], 'binary')
        self.assertEqual(anchors['module']['unsupported'], 'submodule')
        self.assertTrue(report['crlf_preserved'])
        self.assertTrue(report['missing_newline_preserved'])


class DiscussionTests(unittest.TestCase):
    def test_pages_threads_and_each_threads_replies_independently(self):
        calls = []
        def query(kind, cursor, thread=None):
            calls.append((kind, cursor, thread))
            if kind == 'threads':
                return {'nodes': [{'id': 'T1' if cursor is None else 'T2'}],
                        'totalCount': 2,
                        'pageInfo': {'hasNextPage': cursor is None, 'endCursor': 'next'}}
            return {'nodes': [{'id': thread + ('C1' if cursor is None else 'C2')}],
                    'totalCount': 2,
                    'pageInfo': {'hasNextPage': cursor is None, 'endCursor': 'reply'}}
        result = harness.discussions(query)
        self.assertTrue(result['complete'])
        self.assertEqual([c['id'] for c in result['threads'][1]['comments']], ['T2C1', 'T2C2'])
        self.assertEqual(len(calls), 6)

    def test_reports_partial_data_and_cursor_failure_without_claiming_complete(self):
        def query(kind, cursor, thread=None):
            if kind == 'comments':
                raise RuntimeError('access denied')
            return {'nodes': [{'id': 'T1'}], 'totalCount': 3,
                    'pageInfo': {'hasNextPage': True, 'endCursor': None}}
        result = harness.discussions(query)
        self.assertFalse(result['complete'])
        self.assertEqual(len(result['threads']), 1)
        self.assertEqual(result['connections'][0]['expected'], 3)
        self.assertEqual(result['connections'][0]['fetched'], 1)
        self.assertEqual(result['connections'][1]['scope'], 'comments:T1')


class AnchorTests(unittest.TestCase):
    def test_ranges_distinguish_reading_from_inline_and_suggestion_eligibility(self):
        file = {'unsupported': None, 'lines': [
            {'old': 1, 'new': 1, 'kind': 'context'},
            {'old': 2, 'new': None, 'kind': 'delete'},
            {'old': None, 'new': 2, 'kind': 'add'}]}
        self.assertTrue(harness.eligible(file, 'old', 1, 2))
        self.assertTrue(harness.eligible(file, 'new', 1, 2, suggestion=True))
        self.assertFalse(harness.eligible(file, 'old', 2, 2, suggestion=True))
        self.assertFalse(harness.eligible(file, 'new', 3, 3))
        self.assertFalse(harness.eligible(file, 'new', 0, 1))


if __name__ == '__main__':
    unittest.main()
