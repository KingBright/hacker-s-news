import argparse
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import delivery


class Api:
    base = 'https://news.example'
    context = None
    def __init__(self, data): self.data = data
    def call(self, path): return self.data


class DeliveryTests(unittest.TestCase):
    entry = {'title': '节目', 'voice_job_id': 'job'}
    def test_queued_is_not_delivered(self):
        row = delivery.check_output(Api({'status':'queued'}), Api(None), self.entry)
        self.assertEqual(row['state'], 'waiting')

    def test_completed_requires_matching_public_audio(self):
        job = {'status':'completed','target_type':'feed_item','target_id':'id','audio_url':'/audio/a.mp3'}
        with self.assertRaises(ValueError):
            delivery.check_output(Api(job), Api({'audio_url':'/audio/b.mp3','duration_sec':20}), self.entry)

    def test_completed_requires_public_bytes_probe(self):
        job = {'status':'completed','target_type':'feed_item','target_id':'id','audio_url':'/audio/a.mp3'}
        target = {'audio_url':'/audio/a.mp3','duration_sec':20,'status':'published'}
        with self.assertRaises(OSError):
            delivery.check_output(Api(job), Api(target), self.entry, probe=lambda *args: (_ for _ in ()).throw(OSError('offline')))

    def test_report_hides_internal_state_and_requires_delivery(self):
        manifest = {'date':'2026-10-09','edition':'早间','warnings':[]}
        row = {'title':'新闻','voice_job_id':'secret-internal-id','state':'ready','audio_url':'https://news.example/audio/a.mp3','duration_sec':65}
        report = delivery.render_report(manifest,[row],{'state':'ready','audio_url':row['audio_url']},True)
        self.assertIn('1:05',report)
        self.assertNotIn('secret-internal-id',report)
        self.assertIn('内容已上线',report)
        report = delivery.render_report(manifest,[dict(row,state='waiting')],{'state':'missing','description':'周报缺失'},False)
        self.assertIn('交付未完成',report)
        self.assertNotIn('内容已上线',report)

    def test_missing_weekly_prevents_success_even_with_ready_radio(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root); manifest=root/'manifest.json'
            manifest.write_text(json.dumps({'date':'2026-10-09','edition':'早间','outputs':[self.entry]}))
            args=argparse.Namespace(manifest=manifest,out=root/'state.json',report=root/'report.md',wait_seconds=0)
            def save(path,value): Path(path).write_text(json.dumps(value))
            ready=dict(self.entry,state='ready',audio_url='https://news.example/a.mp3',duration_sec=60)
            with patch.object(delivery,'check_output',return_value=ready),patch.object(delivery,'check_weekly',return_value={'state':'missing','description':'周报缺失'}):
                self.assertEqual(delivery.finalize(args,None,None,save),2)
            self.assertFalse(json.loads(args.out.read_text())['delivered'])

    def test_cross_origin_audio_rejected_without_request(self):
        with patch.object(delivery,'urlopen') as request:
            with self.assertRaises(ValueError): delivery.audio_probe('https://news.example','https://other.example/a.mp3',None)
            request.assert_not_called()

    def test_no_new_content_is_not_claimed_as_new_delivery(self):
        report = delivery.render_report({'date':'2026-10-09','edition':'晚间'}, [], {'state':'ready','audio_url':'https://news.example/w.mp3'}, True)
        self.assertIn('无新增内容', report)
        self.assertNotIn('内容已上线，可收听', report)

    def test_monday_weekly_is_not_due_before_eight(self):
        from datetime import datetime
        from zoneinfo import ZoneInfo
        result=delivery.check_weekly(Api(None),now=datetime(2026,10,12,7,30,tzinfo=ZoneInfo('Asia/Shanghai')))
        self.assertEqual(result['state'],'not_due')


if __name__ == '__main__': unittest.main()
