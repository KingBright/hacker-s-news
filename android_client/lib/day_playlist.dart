class DayPlaylistGroup<T> {
  final String key;
  final DateTime dayStart;
  final String title;
  final String shortTitle;
  final List<T> items;
  final List<String> itemIds;
  final List<String> playbackIds;
  final int playableCount;
  final int totalDurationSec;

  const DayPlaylistGroup({
    required this.key,
    required this.dayStart,
    required this.title,
    required this.shortTitle,
    required this.items,
    required this.itemIds,
    required this.playbackIds,
    required this.playableCount,
    required this.totalDurationSec,
  });
}

enum DayPlaylistSortOrder { ascending, descending }

class _DayBucket<T> {
  final String key;
  final DateTime dayStart;
  final List<T> items;

  const _DayBucket({
    required this.key,
    required this.dayStart,
    required this.items,
  });
}

int _compareInts(int left, int right, DayPlaylistSortOrder order) {
  return order == DayPlaylistSortOrder.ascending
      ? left.compareTo(right)
      : right.compareTo(left);
}

String _dayKey(DateTime date) {
  final year = date.year.toString().padLeft(4, '0');
  final month = date.month.toString().padLeft(2, '0');
  final day = date.day.toString().padLeft(2, '0');
  return '$year-$month-$day';
}

DateTime _localDayStart(DateTime date) =>
    DateTime(date.year, date.month, date.day);

({String title, String shortTitle}) _formatDayLabel(
  DateTime dayStart,
  DateTime now,
) {
  final today = _localDayStart(now);
  final yesterday = today.subtract(const Duration(days: 1));
  const weekdays = <int, String>{
    DateTime.monday: '周一',
    DateTime.tuesday: '周二',
    DateTime.wednesday: '周三',
    DateTime.thursday: '周四',
    DateTime.friday: '周五',
    DateTime.saturday: '周六',
    DateTime.sunday: '周日',
  };
  final weekday = weekdays[dayStart.weekday] ?? '';
  final full = '${dayStart.month}月${dayStart.day}日 $weekday';
  final short = '${dayStart.month}月${dayStart.day}日';

  if (dayStart == today) {
    return (title: '今天 · $full', shortTitle: '今天');
  }
  if (dayStart == yesterday) {
    return (title: '昨天 · $full', shortTitle: '昨天');
  }
  return (title: full, shortTitle: short);
}

List<DayPlaylistGroup<T>> buildDayPlaylists<T>({
  required Iterable<T> items,
  required String Function(T item) idOf,
  required int? Function(T item) timestampSecondsOf,
  bool Function(T item)? isPlayable,
  int? Function(T item)? durationSecondsOf,
  DayPlaylistSortOrder dayOrder = DayPlaylistSortOrder.descending,
  DayPlaylistSortOrder itemOrder = DayPlaylistSortOrder.ascending,
  DayPlaylistSortOrder playbackOrder = DayPlaylistSortOrder.ascending,
  DateTime? now,
}) {
  final currentTime = now ?? DateTime.now();
  final canPlay = isPlayable ?? (_) => true;
  final durationOf = durationSecondsOf ?? (_) => 0;
  final buckets = <String, _DayBucket<T>>{};

  for (final item in items) {
    final rawSeconds = timestampSecondsOf(item) ?? 0;
    final timestamp = rawSeconds > 0
        ? DateTime.fromMillisecondsSinceEpoch(rawSeconds * 1000)
        : currentTime;
    final dayStart = _localDayStart(timestamp);
    final key = _dayKey(dayStart);
    final bucket = buckets[key];
    if (bucket != null) {
      bucket.items.add(item);
    } else {
      buckets[key] = _DayBucket<T>(key: key, dayStart: dayStart, items: [item]);
    }
  }

  final sortedBuckets = buckets.values.toList()
    ..sort(
      (left, right) => _compareInts(
        left.dayStart.millisecondsSinceEpoch,
        right.dayStart.millisecondsSinceEpoch,
        dayOrder,
      ),
    );

  return sortedBuckets
      .map((bucket) {
        final orderedItems = [...bucket.items]
          ..sort((left, right) {
            final leftTs = timestampSecondsOf(left) ?? 0;
            final rightTs = timestampSecondsOf(right) ?? 0;
            final byTime = _compareInts(leftTs, rightTs, itemOrder);
            if (byTime != 0) return byTime;
            return idOf(left).compareTo(idOf(right));
          });

        final playbackItems = bucket.items.where(canPlay).toList()
          ..sort((left, right) {
            final leftTs = timestampSecondsOf(left) ?? 0;
            final rightTs = timestampSecondsOf(right) ?? 0;
            final byTime = _compareInts(leftTs, rightTs, playbackOrder);
            if (byTime != 0) return byTime;
            return idOf(left).compareTo(idOf(right));
          });

        final labels = _formatDayLabel(bucket.dayStart, currentTime);
        return DayPlaylistGroup<T>(
          key: bucket.key,
          dayStart: bucket.dayStart,
          title: labels.title,
          shortTitle: labels.shortTitle,
          items: orderedItems,
          itemIds: orderedItems.map(idOf).toList(growable: false),
          playbackIds: playbackItems.map(idOf).toList(growable: false),
          playableCount: playbackItems.length,
          totalDurationSec: bucket.items.fold<int>(0, (total, item) {
            final duration = durationOf(item) ?? 0;
            return total + (duration > 0 ? duration : 0);
          }),
        );
      })
      .toList(growable: false);
}
