import 'dart:convert';

import 'package:http/http.dart' as http;

class CuratedFeedItem {
  final String id;
  final String title;
  final String? subtitle;
  final String? sourceName;
  final String? originalUrl;
  final int? publishTime;
  final bool hasAudio;
  final String? audioUrl;
  final int? durationSec;
  final int? readingTimeMin;
  final int? qualityScore;
  final String? tags;

  CuratedFeedItem({
    required this.id,
    required this.title,
    this.subtitle,
    this.sourceName,
    this.originalUrl,
    this.publishTime,
    required this.hasAudio,
    this.audioUrl,
    this.durationSec,
    this.readingTimeMin,
    this.qualityScore,
    this.tags,
  });

  factory CuratedFeedItem.fromJson(Map<String, dynamic> json) {
    final audioUrl = json['audio_url'] as String?;
    return CuratedFeedItem(
      id: json['id'] as String,
      title: json['title'] as String? ?? 'Untitled',
      subtitle: json['subtitle'] as String?,
      sourceName: json['source_name'] as String?,
      originalUrl: json['original_url'] as String?,
      publishTime: _asInt(json['publish_time']),
      hasAudio: audioUrl != null && audioUrl.isNotEmpty,
      audioUrl: audioUrl,
      durationSec: _asInt(json['duration_sec']),
      readingTimeMin: _asInt(json['reading_time_min']),
      qualityScore: _asInt(json['quality_score']),
      tags: json['tags'] as String?,
    );
  }
}

class CuratedFeedContent {
  final String itemId;
  final String? readerMarkdown;
  final String? plainText;
  final String? compressedMarkdown;
  final String? audioScript;
  final String? keyPointsJson;

  CuratedFeedContent({
    required this.itemId,
    this.readerMarkdown,
    this.plainText,
    this.compressedMarkdown,
    this.audioScript,
    this.keyPointsJson,
  });

  factory CuratedFeedContent.fromJson(Map<String, dynamic> json) {
    return CuratedFeedContent(
      itemId: json['item_id'] as String? ?? '',
      readerMarkdown: json['reader_markdown'] as String?,
      plainText: json['plain_text'] as String?,
      compressedMarkdown: json['compressed_markdown'] as String?,
      audioScript: json['audio_script'] as String?,
      keyPointsJson: json['key_points_json'] as String?,
    );
  }

  String textForMode(ReadingMode mode) {
    if (mode == ReadingMode.compressed) {
      final compressed = compressedMarkdown?.trim();
      if (compressed != null && compressed.isNotEmpty) return compressed;
      final audio = audioScript?.trim();
      if (audio != null && audio.isNotEmpty) return audio;
      final points = parseStringList(keyPointsJson);
      if (points.isNotEmpty) {
        return points.map((point) => '- $point').join('\n');
      }
      final plain = plainText?.trim();
      if (plain != null && plain.isNotEmpty) return plain;
      return '这篇文章还没有生成干货压缩。';
    }

    final markdown = readerMarkdown?.trim();
    if (markdown != null && markdown.isNotEmpty) return markdown;
    return plainText?.trim() ?? '';
  }
}

class WeeklyDigest {
  final String id;
  final String title;
  final int weekStart;
  final int weekEnd;
  final String? digestMarkdown;
  final String? audioScript;
  final String? audioUrl;
  final int? durationSec;
  final String? themesJson;

  WeeklyDigest({
    required this.id,
    required this.title,
    required this.weekStart,
    required this.weekEnd,
    this.digestMarkdown,
    this.audioScript,
    this.audioUrl,
    this.durationSec,
    this.themesJson,
  });

  factory WeeklyDigest.fromJson(Map<String, dynamic> json) {
    return WeeklyDigest(
      id: json['id'] as String,
      title: json['title'] as String? ?? 'FreshLoop Weekly',
      weekStart: _asInt(json['week_start']) ?? 0,
      weekEnd: _asInt(json['week_end']) ?? 0,
      digestMarkdown: json['digest_markdown'] as String?,
      audioScript: json['audio_script'] as String?,
      audioUrl: json['audio_url'] as String?,
      durationSec: _asInt(json['duration_sec']),
      themesJson: json['themes_json'] as String?,
    );
  }
}

enum ReadingMode { original, compressed }

class CuratedFeedApi {
  final String baseUrl;
  final http.Client _client;

  CuratedFeedApi({required this.baseUrl, http.Client? client})
    : _client = client ?? http.Client();

  Future<List<CuratedFeedItem>> fetchItems({int limit = 40}) async {
    final uri = Uri.parse(
      '$baseUrl/api/feed/items?product_line=curated_feed&item_type=article&limit=$limit',
    );
    final response = await _client.get(uri);
    _ensureSuccess(response);
    final data = jsonDecode(response.body);
    if (data is! List) return [];
    return data
        .whereType<Map<String, dynamic>>()
        .map(CuratedFeedItem.fromJson)
        .toList();
  }

  Future<CuratedFeedContent?> fetchContent(String itemId) async {
    final uri = Uri.parse('$baseUrl/api/feed/items/$itemId/content');
    final response = await _client.get(uri);
    if (response.statusCode == 404) return null;
    _ensureSuccess(response);
    final data = jsonDecode(response.body);
    if (data is! Map<String, dynamic>) return null;
    return CuratedFeedContent.fromJson(data);
  }

  Future<List<WeeklyDigest>> fetchWeeklies() async {
    final uri = Uri.parse('$baseUrl/api/feed/weeklies');
    final response = await _client.get(uri);
    _ensureSuccess(response);
    final data = jsonDecode(response.body);
    if (data is! List) return [];
    return data
        .whereType<Map<String, dynamic>>()
        .map(WeeklyDigest.fromJson)
        .toList();
  }

  Future<void> saveProgress({
    required String itemId,
    required ReadingMode mode,
    required double scrollRatio,
    required String? userId,
  }) async {
    if (userId == null || userId.isEmpty) return;
    final uri = Uri.parse('$baseUrl/api/feed/items/$itemId/progress');
    await _client.post(
      uri,
      headers: {'Content-Type': 'application/json', 'x-user-id': userId},
      body: jsonEncode({
        'mode': mode == ReadingMode.original ? 'original' : 'compressed',
        'scroll_ratio': scrollRatio.clamp(0.0, 1.0),
      }),
    );
  }

  void _ensureSuccess(http.Response response) {
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw Exception('HTTP ${response.statusCode}: ${response.body}');
    }
  }
}

int? _asInt(dynamic value) {
  if (value is int) return value;
  if (value is num) return value.toInt();
  return null;
}

List<String> parseStringList(String? value) {
  if (value == null || value.trim().isEmpty) return [];
  try {
    final decoded = jsonDecode(value);
    if (decoded is! List) return [];
    return decoded.whereType<String>().toList();
  } catch (_) {
    return [];
  }
}
