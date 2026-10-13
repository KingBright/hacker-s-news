"""Known response, partial report and unknown invocation transport checks."""
import tempfile,unittest
from pathlib import Path
from unittest.mock import Mock
from freshloop_agent import save,read
from refresh_receipt import refresh_once
from review_comparison import EditorialError

class RefreshReceiptTests(unittest.TestCase):
    def test_completed_report_preserved_and_second_invocation_is_read_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'refresh.json';api=Mock();result=dict(feeds_ok=118,entries_cached=1270,errors=[]);api.call.return_value=result
            self.assertEqual(refresh_once(api,p,save,read),result);self.assertEqual(refresh_once(api,p,save,read),result)
            api.call.assert_called_once_with('/api/agent/sources/refresh',{},retry=False)
            self.assertEqual(read(p),result);p.write_text('{}')
            with self.assertRaisesRegex(EditorialError,'changed'):refresh_once(api,p,save,read)
    def test_partial_report_retains_actual_errors_and_blocks_full_coverage(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'refresh.json';api=Mock();api.call.return_value=dict(feeds_ok=117,entries_cached=1100,errors=['actual feed failed'])
            for _ in range(2):
                with self.assertRaisesRegex(EditorialError,'feed failures'):refresh_once(api,p,save,read)
            self.assertEqual(read(p)['errors'],['actual feed failed']);api.call.assert_called_once()
            self.assertEqual(read(p.with_suffix('.json.checkpoint.json'))['outcome'],'partial')
    def test_unknown_result_never_replays_across_invocations(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'refresh.json';api=Mock();api.call.side_effect=RuntimeError('Request outcome unknown')
            with self.assertRaisesRegex(RuntimeError,'unknown'):refresh_once(api,p,save,read)
            with self.assertRaisesRegex(EditorialError,'do not replay'):refresh_once(api,p,save,read)
            api.call.assert_called_once();self.assertFalse(p.exists())
            self.assertEqual(read(p.with_suffix('.json.checkpoint.json'))['outcome'],'unknown')
    def test_new_output_path_cannot_replay_unresolved_run_refresh(self):
        import argparse
        from review_comparison import record_issue
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'first.json';api=Mock()
            record_issue(argparse.Namespace(command='refresh',out=str(p)),RuntimeError('unknown original refresh'))
            with self.assertRaisesRegex(EditorialError,'new output path'):refresh_once(api,p.with_name('second.json'),save,read)
            api.call.assert_not_called()
if __name__=='__main__':unittest.main()
