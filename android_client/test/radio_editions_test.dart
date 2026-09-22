import 'package:flutter_test/flutter_test.dart';
import 'package:android_client/radio_editions.dart';
import 'package:android_client/src/rust/models.dart';

void main() {
  test('complete program replaces old category tracks once audio is ready', () {
    const old=Item(id:'old',title:'old',audioUrl:'/audio/old.mp3',durationSec:60,tags:'["radio:date:2026-09-22","radio:edition:morning"]');
    const full=Item(id:'full',title:'full',audioUrl:'/audio/full.mp3',durationSec:600,tags:'["radio:date:2026-09-22","radio:edition:morning","radio:program","radio:sections:11"]');
    final group=buildRadioEditions([old,full]).single;
    expect(group.playbackIds,['full']);expect(group.totalDurationSec,600);expect(programSectionCount(full),11);
  });
  test(
    'Logical morning survives midnight TTS and keeps waiting audio out of totals',
    () {
      const ready = Item(
        id: 'a',
        title: 'Morning',
        publishTime: 1790006400,
        audioUrl: '/audio/a.mp3',
        durationSec: 61,
        tags: '["radio:date:2026-09-21","radio:edition:morning"]',
      );
      const waiting = Item(
        id: 'b',
        title: 'Waiting',
        publishTime: 1790006400,
        durationSec: 999,
        tags: '["radio:date:2026-09-21","radio:edition:morning"]',
      );
      const evening = Item(
        id: 'c',
        title: 'Evening',
        publishTime: 1790006400,
        audioUrl: '/audio/c.mp3',
        durationSec: 90,
        tags: '["radio:date:2026-09-21","radio:edition:evening"]',
      );
      final groups = buildRadioEditions([ready, waiting, evening]);
      expect(groups.length, 2);
      final morning = groups.singleWhere((g) => g.key.endsWith('morning'));
      expect(morning.title, contains('2026年9月21日 星期一'));
      expect(morning.playbackIds, ['a']);
      expect(morning.totalDurationSec, 61);
      expect(morning.items.length, 2);
    },
  );
  test(
    'Legacy timestamps follow Shanghai date, independent of device timezone',
    () {
      final item = Item(
        id: 'a',
        title: 'Legacy',
        publishTime:
            DateTime.parse('2026-09-20T23:30:00Z').millisecondsSinceEpoch ~/
            1000,
      );
      expect(buildRadioEditions([item]).single.key, '2026-09-21:morning');
    },
  );
}
