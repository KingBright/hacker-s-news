import 'dart:convert';

import 'package:http/http.dart' as http;

class LoopPostReference {
  final String id;
  final String? sourceType;
  final String? sourceId;
  final String? sourceUrl;
  final String? title;
  final String? quoteText;

  const LoopPostReference({
    required this.id,
    this.sourceType,
    this.sourceId,
    this.sourceUrl,
    this.title,
    this.quoteText,
  });

  factory LoopPostReference.fromJson(Map<String, dynamic> json) {
    return LoopPostReference(
      id: json['id'] as String? ?? '',
      sourceType: json['source_type'] as String?,
      sourceId: json['source_id'] as String?,
      sourceUrl: json['source_url'] as String?,
      title: json['title'] as String?,
      quoteText: json['quote_text'] as String?,
    );
  }
}

class LoopPostEntry {
  final String id;
  final String postType;
  final String feedbackMode;
  final String? title;
  final String body;
  final String? preferenceStatus;
  final int? createdAt;
  final List<LoopPostReference> references;

  const LoopPostEntry({
    required this.id,
    required this.postType,
    required this.feedbackMode,
    this.title,
    required this.body,
    this.preferenceStatus,
    this.createdAt,
    this.references = const [],
  });

  factory LoopPostEntry.fromJson(Map<String, dynamic> json) {
    final post = json['post'] as Map<String, dynamic>? ?? json;
    final references =
        (json['references'] as List?)
            ?.whereType<Map<String, dynamic>>()
            .map(LoopPostReference.fromJson)
            .toList() ??
        const <LoopPostReference>[];

    return LoopPostEntry(
      id: post['id'] as String? ?? '',
      postType: post['post_type'] as String? ?? 'thought',
      feedbackMode: post['feedback_mode'] as String? ?? 'balance',
      title: post['title'] as String?,
      body: post['body'] as String? ?? '',
      preferenceStatus: post['preference_status'] as String?,
      createdAt: _asInt(post['created_at']),
      references: references,
    );
  }
}

class FocusCard {
  final String label;
  final String kind;
  final double score;
  final String evidence;

  const FocusCard({
    required this.label,
    required this.kind,
    required this.score,
    required this.evidence,
  });

  factory FocusCard.fromJson(Map<String, dynamic> json) {
    return FocusCard(
      label: json['label'] as String? ?? '',
      kind: json['kind'] as String? ?? '',
      score: (json['score'] as num?)?.toDouble() ?? 0,
      evidence: json['evidence'] as String? ?? '',
    );
  }
}

class BalanceRule {
  final int activePct;
  final int stablePct;
  final int explorePct;
  final String note;

  const BalanceRule({
    required this.activePct,
    required this.stablePct,
    required this.explorePct,
    required this.note,
  });

  factory BalanceRule.fromJson(Map<String, dynamic> json) {
    return BalanceRule(
      activePct: _asInt(json['active_pct']) ?? 0,
      stablePct: _asInt(json['stable_pct']) ?? 0,
      explorePct: _asInt(json['explore_pct']) ?? 0,
      note: json['note'] as String? ?? '',
    );
  }
}

class FocusStats {
  final int expressionCount;
  final int processedExpressionCount;
  final int pendingExpressionCount;
  final int signalCount;

  const FocusStats({
    required this.expressionCount,
    required this.processedExpressionCount,
    required this.pendingExpressionCount,
    required this.signalCount,
  });

  factory FocusStats.fromJson(Map<String, dynamic> json) {
    return FocusStats(
      expressionCount: _asInt(json['expression_count']) ?? 0,
      processedExpressionCount: _asInt(json['processed_expression_count']) ?? 0,
      pendingExpressionCount: _asInt(json['pending_expression_count']) ?? 0,
      signalCount: _asInt(json['signal_count']) ?? 0,
    );
  }
}

class FocusSummary {
  final List<FocusCard> currentFocus;
  final List<FocusCard> longTermFocus;
  final List<FocusCard> recentlyReduced;
  final List<FocusCard> preferredSources;
  final List<FocusCard> preferredFormats;
  final BalanceRule readingBalance;
  final BalanceRule radioBalance;
  final FocusStats stats;
  final String note;

  const FocusSummary({
    required this.currentFocus,
    required this.longTermFocus,
    required this.recentlyReduced,
    required this.preferredSources,
    required this.preferredFormats,
    required this.readingBalance,
    required this.radioBalance,
    required this.stats,
    required this.note,
  });

  factory FocusSummary.fromJson(Map<String, dynamic> json) {
    List<FocusCard> parseCards(String key) =>
        (json[key] as List?)
            ?.whereType<Map<String, dynamic>>()
            .map(FocusCard.fromJson)
            .toList() ??
        const <FocusCard>[];

    return FocusSummary(
      currentFocus: parseCards('current_focus'),
      longTermFocus: parseCards('long_term_focus'),
      recentlyReduced: parseCards('recently_reduced'),
      preferredSources: parseCards('preferred_sources'),
      preferredFormats: parseCards('preferred_formats'),
      readingBalance: BalanceRule.fromJson(
        json['reading_balance'] as Map<String, dynamic>? ?? const {},
      ),
      radioBalance: BalanceRule.fromJson(
        json['radio_balance'] as Map<String, dynamic>? ?? const {},
      ),
      stats: FocusStats.fromJson(
        json['stats'] as Map<String, dynamic>? ?? const {},
      ),
      note: json['note'] as String? ?? '',
    );
  }
}

class LoopApi {
  final String baseUrl;
  final http.Client _client;

  LoopApi({required this.baseUrl, http.Client? client})
    : _client = client ?? http.Client();

  Future<List<LoopPostEntry>> fetchPosts(
    String userId, {
    int limit = 60,
  }) async {
    final uri = Uri.parse('$baseUrl/api/loop/posts?limit=$limit');
    final response = await _client.get(uri, headers: {'x-user-id': userId});
    _ensureSuccess(response);
    final data = jsonDecode(response.body);
    if (data is! List) return [];
    return data
        .whereType<Map<String, dynamic>>()
        .map(LoopPostEntry.fromJson)
        .toList();
  }

  Future<LoopPostEntry> createPost({
    required String userId,
    required String body,
    required String feedbackMode,
    String? title,
    String? sourceRef,
    List<Map<String, dynamic>> references = const [],
  }) async {
    final uri = Uri.parse('$baseUrl/api/loop/posts');
    final response = await _client.post(
      uri,
      headers: {'Content-Type': 'application/json', 'x-user-id': userId},
      body: jsonEncode({
        'post_type': references.isEmpty ? 'thought' : 'quote_comment',
        'feedback_mode': feedbackMode,
        'title': title,
        'body': body,
        'visibility': 'private',
        'source_ref': sourceRef,
        'references': references,
      }),
    );
    _ensureSuccess(response);
    final data = jsonDecode(response.body);
    if (data is! Map<String, dynamic>) {
      throw Exception('Unexpected response');
    }
    return LoopPostEntry.fromJson(data);
  }

  Future<FocusSummary> fetchFocus(String userId) async {
    final uri = Uri.parse('$baseUrl/api/focus');
    final response = await _client.get(uri, headers: {'x-user-id': userId});
    _ensureSuccess(response);
    final data = jsonDecode(response.body);
    if (data is! Map<String, dynamic>) {
      throw Exception('Unexpected response');
    }
    return FocusSummary.fromJson(data);
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

String? loopPreferenceStatusLabel(String? status) {
  switch (status) {
    case 'processed':
      return '已吸收';
    case 'pending':
      return '待整理';
    case 'failed':
      return '整理失败';
    case 'skipped':
      return '已略过';
    default:
      return null;
  }
}

String focusKindLabel(String? kind) {
  switch (kind) {
    case 'topic':
      return '主题';
    case 'source':
      return '来源';
    case 'signal':
      return '偏好';
    case 'format':
      return '形态';
    default:
      return kind ?? '';
  }
}
