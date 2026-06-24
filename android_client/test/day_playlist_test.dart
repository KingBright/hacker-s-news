import 'package:android_client/day_playlist.dart';
import 'package:flutter_test/flutter_test.dart';

class _FakeEntry {
  final String id;
  final int timestamp;
  final int durationSec;
  final bool playable;

  const _FakeEntry({
    required this.id,
    required this.timestamp,
    required this.durationSec,
    required this.playable,
  });
}

void main() {
  test(
    'buildDayPlaylists groups by day and keeps playback order chronological',
    () {
      final items = [
        _FakeEntry(
          id: 'b',
          timestamp:
              DateTime(2026, 6, 24, 10, 30).millisecondsSinceEpoch ~/ 1000,
          durationSec: 120,
          playable: true,
        ),
        _FakeEntry(
          id: 'a',
          timestamp: DateTime(2026, 6, 24, 8, 0).millisecondsSinceEpoch ~/ 1000,
          durationSec: 60,
          playable: false,
        ),
        _FakeEntry(
          id: 'c',
          timestamp:
              DateTime(2026, 6, 23, 21, 15).millisecondsSinceEpoch ~/ 1000,
          durationSec: 180,
          playable: true,
        ),
      ];

      final groups = buildDayPlaylists<_FakeEntry>(
        items: items,
        idOf: (item) => item.id,
        timestampSecondsOf: (item) => item.timestamp,
        isPlayable: (item) => item.playable,
        durationSecondsOf: (item) => item.durationSec,
        now: DateTime(2026, 6, 24, 12, 0),
      );

      expect(groups, hasLength(2));
      expect(groups.first.shortTitle, '今天');
      expect(groups.first.itemIds, ['a', 'b']);
      expect(groups.first.playbackIds, ['b']);
      expect(groups.first.totalDurationSec, 180);
      expect(groups[1].shortTitle, '昨天');
      expect(groups[1].playbackIds, ['c']);
    },
  );

  test('buildDayPlaylists can separate visual order from playback order', () {
    final items = [
      _FakeEntry(
        id: 'morning',
        timestamp: DateTime(2026, 6, 24, 8, 0).millisecondsSinceEpoch ~/ 1000,
        durationSec: 60,
        playable: true,
      ),
      _FakeEntry(
        id: 'evening',
        timestamp: DateTime(2026, 6, 24, 20, 0).millisecondsSinceEpoch ~/ 1000,
        durationSec: 120,
        playable: true,
      ),
      _FakeEntry(
        id: 'noon',
        timestamp: DateTime(2026, 6, 24, 12, 0).millisecondsSinceEpoch ~/ 1000,
        durationSec: 90,
        playable: true,
      ),
    ];

    final groups = buildDayPlaylists<_FakeEntry>(
      items: items,
      idOf: (item) => item.id,
      timestampSecondsOf: (item) => item.timestamp,
      isPlayable: (item) => item.playable,
      itemOrder: DayPlaylistSortOrder.descending,
      playbackOrder: DayPlaylistSortOrder.ascending,
      now: DateTime(2026, 6, 24, 21, 0),
    );

    expect(groups.single.itemIds, ['evening', 'noon', 'morning']);
    expect(groups.single.playbackIds, ['morning', 'noon', 'evening']);
  });
}
