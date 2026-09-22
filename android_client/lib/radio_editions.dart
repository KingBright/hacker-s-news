import 'dart:convert';
import 'day_playlist.dart';
import 'src/rust/models.dart';

({String key, String date, String edition}) radioEdition(Item item) {
  List<String> tags = [];
  try {
    final parsed = jsonDecode(item.tags ?? '[]');
    if (parsed is List) tags = parsed.whereType<String>().toList();
  } on FormatException {
    /* Historical tags can be plain text. */
  }
  final local = DateTime.fromMillisecondsSinceEpoch(
    (item.publishTime ?? item.createdAt ?? 0) * 1000,
    isUtc: true,
  ).add(const Duration(hours: 8));
  final dates = tags.where(
    (t) => RegExp(r'^radio:date:\d{4}-\d{2}-\d{2}$').hasMatch(t),
  );
  final date = dates.isNotEmpty
      ? dates.first.substring(11)
      : local.toIso8601String().substring(0, 10);
  final edition = tags.contains('radio:edition:morning')
      ? 'morning'
      : tags.contains('radio:edition:evening')
      ? 'evening'
      : local.hour < 12
      ? 'morning'
      : 'evening';
  return (key: '$date:$edition', date: date, edition: edition);
}

List<DayPlaylistGroup<Item>> buildRadioEditions(Iterable<Item> items) {
  final buckets = <String, List<Item>>{};
  for (final item in items) {
    (buckets[radioEdition(item).key] ??= []).add(item);
  }
  final result =
      buckets.entries.map((entry) {
        final meta = radioEdition(entry.value.first);
        final date = DateTime.parse('${meta.date}T00:00:00Z');
        const weekdays = ['星期一', '星期二', '星期三', '星期四', '星期五', '星期六', '星期日'];
        final label = meta.edition == 'morning' ? '早间新闻' : '晚间新闻';
        final programs = entry.value.where((i) => isRadioProgram(i) && (i.audioUrl?.trim().isNotEmpty ?? false)).toList();
        final ordered = [...(programs.isNotEmpty ? programs : entry.value)]
          ..sort((a, b) {
            final delta = (a.publishTime ?? a.createdAt ?? 0).compareTo(
              b.publishTime ?? b.createdAt ?? 0,
            );
            return delta == 0 ? a.id.compareTo(b.id) : delta;
          });
        final playable = ordered
            .where((i) => i.audioUrl?.trim().isNotEmpty ?? false)
            .toList();
        return DayPlaylistGroup<Item>(
          key: entry.key,
          dayStart: date,
          title:
              '$label · ${date.year}年${date.month}月${date.day}日 ${weekdays[date.weekday - 1]}',
          shortTitle: label,
          items: ordered,
          itemIds: ordered.map((i) => i.id).toList(),
          playbackIds: playable.map((i) => i.id).toList(),
          playableCount: playable.length,
          totalDurationSec: playable.fold(
            0,
            (n, i) => n + ((i.durationSec ?? 0) > 0 ? i.durationSec! : 0),
          ),
        );
      }).toList()..sort((a, b) {
        final delta = b.dayStart.compareTo(a.dayStart);
        return delta == 0 ? a.key.compareTo(b.key) : delta;
      });
  return result;
}

bool isRadioProgram(Item item) {
  try { final tags=jsonDecode(item.tags ?? '[]'); return tags is List && tags.contains('radio:program'); } on FormatException { return false; }
}
int programSectionCount(Item item) {
  try { final tags=jsonDecode(item.tags ?? '[]'); if(tags is List) { for(final tag in tags.whereType<String>()) { if(tag.startsWith('radio:sections:')) return int.tryParse(tag.substring(15)) ?? 0; } } } on FormatException { return 0; }
  return 0;
}
