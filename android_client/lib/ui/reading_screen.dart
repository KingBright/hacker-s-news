import 'dart:async';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../feed_api.dart';
import '../main.dart';
import '../src/rust/models.dart';
import 'theme.dart';

class ReadingFeedProvider extends ChangeNotifier {
  final CuratedFeedApi api;
  List<CuratedFeedItem> items = [];
  List<WeeklyDigest> weeklies = [];
  final Map<String, CuratedFeedContent> _contentCache = {};
  bool isLoading = false;
  String? error;

  ReadingFeedProvider(this.api) {
    refresh();
  }

  Future<void> refresh() async {
    if (isLoading) return;
    isLoading = true;
    error = null;
    notifyListeners();

    try {
      final itemsFuture = api.fetchItems();
      final weekliesFuture = api.fetchWeeklies();
      items = await itemsFuture;
      weeklies = await weekliesFuture;
    } catch (e) {
      error = e.toString();
    } finally {
      isLoading = false;
      notifyListeners();
    }
  }

  Future<CuratedFeedContent?> loadContent(String itemId) async {
    final cached = _contentCache[itemId];
    if (cached != null) return cached;
    final content = await api.fetchContent(itemId);
    if (content != null) {
      _contentCache[itemId] = content;
      notifyListeners();
    }
    return content;
  }
}

class ReadingScreen extends StatelessWidget {
  const ReadingScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Consumer<ReadingFeedProvider>(
      builder: (context, provider, child) {
        return RefreshIndicator(
          color: AppTheme.primaryGreen,
          backgroundColor: AppTheme.surfaceDark,
          onRefresh: provider.refresh,
          child: ListView(
            padding: const EdgeInsets.fromLTRB(16, 12, 16, 120),
            children: [
              _WeeklySection(weeklies: provider.weeklies),
              const SizedBox(height: 18),
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  const Text(
                    '精选阅读',
                    style: TextStyle(fontSize: 22, fontWeight: FontWeight.w900),
                  ),
                  if (provider.isLoading)
                    const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        color: AppTheme.primaryGreen,
                      ),
                    ),
                ],
              ),
              const SizedBox(height: 12),
              if (provider.error != null)
                _EmptyPanel(
                  icon: Icons.cloud_off_outlined,
                  text: '精选频道暂时不可用',
                  detail: provider.error,
                )
              else if (provider.items.isEmpty && !provider.isLoading)
                const _EmptyPanel(icon: Icons.article_outlined, text: '暂无精选文章')
              else
                ...provider.items.map((item) => _ArticleCard(item: item)),
            ],
          ),
        );
      },
    );
  }
}

class _WeeklySection extends StatelessWidget {
  final List<WeeklyDigest> weeklies;

  const _WeeklySection({required this.weeklies});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: const Color(0xFF102B36),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: Colors.white.withValues(alpha: 0.08)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Row(
            children: [
              Icon(Icons.graphic_eq, color: AppTheme.primaryGreen, size: 18),
              SizedBox(width: 8),
              Text(
                '本周精选汇总',
                style: TextStyle(fontWeight: FontWeight.w900, fontSize: 16),
              ),
            ],
          ),
          const SizedBox(height: 12),
          if (weeklies.isEmpty)
            const Text(
              '周汇总文稿生成后会出现在这里',
              style: TextStyle(color: Colors.white54, height: 1.5),
            )
          else
            ...weeklies.take(3).map((weekly) => _WeeklyCard(weekly: weekly)),
        ],
      ),
    );
  }
}

class _WeeklyCard extends StatelessWidget {
  final WeeklyDigest weekly;

  const _WeeklyCard({required this.weekly});

  @override
  Widget build(BuildContext context) {
    final dateRange =
        '${_formatDate(weekly.weekStart)} - ${_formatDate(weekly.weekEnd)}';
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: InkWell(
        borderRadius: BorderRadius.circular(8),
        onTap: () {
          Navigator.of(context).push(
            MaterialPageRoute(builder: (_) => WeeklyDigestPage(weekly: weekly)),
          );
        },
        child: Container(
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            color: Colors.white.withValues(alpha: 0.08),
            borderRadius: BorderRadius.circular(8),
          ),
          child: Row(
            children: [
              const Icon(Icons.newspaper, color: AppTheme.primaryGreen),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      weekly.title,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontWeight: FontWeight.w800),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      weekly.audioUrl == null || weekly.audioUrl!.isEmpty
                          ? '$dateRange · 文稿'
                          : '$dateRange · ${_duration(weekly.durationSec)}',
                      style: const TextStyle(
                        fontSize: 12,
                        color: Colors.white54,
                      ),
                    ),
                  ],
                ),
              ),
              if (weekly.audioUrl != null && weekly.audioUrl!.isNotEmpty)
                IconButton(
                  onPressed: () => _playSingleItem(_weeklyToAudioItem(weekly)),
                  icon: const Icon(Icons.play_circle_fill),
                  color: AppTheme.primaryGreen,
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class WeeklyDigestPage extends StatelessWidget {
  final WeeklyDigest weekly;

  const WeeklyDigestPage({super.key, required this.weekly});

  @override
  Widget build(BuildContext context) {
    final body = (weekly.digestMarkdown?.trim().isNotEmpty ?? false)
        ? weekly.digestMarkdown!.trim()
        : (weekly.audioScript?.trim() ?? '');
    final hasAudio = weekly.audioUrl != null && weekly.audioUrl!.isNotEmpty;

    return Scaffold(
      backgroundColor: AppTheme.darkBackground,
      appBar: AppBar(
        backgroundColor: AppTheme.darkBackground,
        foregroundColor: Colors.white,
        title: const Text(
          '本周精选汇总',
          style: TextStyle(fontSize: 15, fontWeight: FontWeight.w900),
        ),
        actions: [
          if (hasAudio)
            IconButton(
              onPressed: () => _playSingleItem(_weeklyToAudioItem(weekly)),
              icon: const Icon(Icons.play_arrow_rounded),
            ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(22, 10, 22, 40),
        children: [
          Text(
            weekly.title,
            style: const TextStyle(
              color: Colors.white,
              fontSize: 26,
              height: 1.12,
              fontWeight: FontWeight.w900,
            ),
          ),
          const SizedBox(height: 12),
          Wrap(
            spacing: 8,
            runSpacing: 6,
            children: [
              _ReaderChip(
                text:
                    '${_formatDate(weekly.weekStart)} - ${_formatDate(weekly.weekEnd)}',
              ),
              if (hasAudio) _ReaderChip(text: _duration(weekly.durationSec)),
            ],
          ),
          const SizedBox(height: 18),
          if (body.isEmpty)
            const Text(
              '周汇总文稿还在生成中',
              style: TextStyle(color: Colors.white54, height: 1.6),
            )
          else
            ..._renderMarkdown(body),
        ],
      ),
    );
  }
}

class _ArticleCard extends StatelessWidget {
  final CuratedFeedItem item;

  const _ArticleCard({required this.item});

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.only(bottom: 10),
      decoration: BoxDecoration(
        color: AppTheme.surfaceDark,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: Colors.white.withValues(alpha: 0.06)),
      ),
      child: InkWell(
        borderRadius: BorderRadius.circular(8),
        onTap: () {
          Navigator.of(
            context,
          ).push(MaterialPageRoute(builder: (_) => ReaderPage(item: item)));
        },
        child: Padding(
          padding: const EdgeInsets.all(14),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Container(
                width: 46,
                height: 46,
                decoration: BoxDecoration(
                  color: const Color(0xFF223E51),
                  borderRadius: BorderRadius.circular(8),
                ),
                child: Icon(
                  item.hasAudio ? Icons.headphones : Icons.article_outlined,
                  color: item.hasAudio ? AppTheme.primaryGreen : Colors.white70,
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      _cleanTitle(item.title),
                      maxLines: 3,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 16,
                        height: 1.25,
                        fontWeight: FontWeight.w900,
                      ),
                    ),
                    if (item.subtitle != null && item.subtitle!.isNotEmpty)
                      Padding(
                        padding: const EdgeInsets.only(top: 6),
                        child: Text(
                          item.subtitle!,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(
                            fontSize: 12,
                            color: AppTheme.textMuted,
                          ),
                        ),
                      ),
                    const SizedBox(height: 9),
                    Wrap(
                      spacing: 8,
                      runSpacing: 4,
                      children: [
                        _MetaChip(text: item.sourceName ?? 'FreshLoop'),
                        _MetaChip(text: _formatDate(item.publishTime)),
                        _MetaChip(text: '${item.readingTimeMin ?? 1} min'),
                        if (item.qualityScore != null)
                          _MetaChip(text: '${item.qualityScore}/10'),
                      ],
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              const Icon(Icons.chevron_right, color: Colors.white38),
            ],
          ),
        ),
      ),
    );
  }
}

class ReaderPage extends StatefulWidget {
  final CuratedFeedItem item;

  const ReaderPage({super.key, required this.item});

  @override
  State<ReaderPage> createState() => _ReaderPageState();
}

class _ReaderPageState extends State<ReaderPage> {
  final ScrollController _scrollController = ScrollController();
  ReadingMode _mode = ReadingMode.original;
  CuratedFeedContent? _content;
  bool _loading = true;
  Timer? _saveTimer;
  SharedPreferences? _prefs;

  @override
  void initState() {
    super.initState();
    _scrollController.addListener(_scheduleSave);
    _load();
  }

  @override
  void dispose() {
    _saveTimer?.cancel();
    _scrollController.removeListener(_scheduleSave);
    _scrollController.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final provider = context.read<ReadingFeedProvider>();
    _prefs = await SharedPreferences.getInstance();
    final content = await provider.loadContent(widget.item.id);
    if (!mounted) return;
    setState(() {
      _content = content;
      _loading = false;
    });
    WidgetsBinding.instance.addPostFrameCallback((_) => _restoreProgress());
  }

  void _switchMode(ReadingMode mode) {
    if (_mode == mode) return;
    _saveProgress();
    setState(() => _mode = mode);
    WidgetsBinding.instance.addPostFrameCallback((_) => _restoreProgress());
  }

  void _restoreProgress() {
    final prefs = _prefs;
    if (prefs == null || !_scrollController.hasClients) return;
    final ratio = prefs.getDouble(_progressKey) ?? 0;
    if (ratio <= 0) return;
    final maxScroll = _scrollController.position.maxScrollExtent;
    _scrollController.jumpTo((maxScroll * ratio).clamp(0.0, maxScroll));
  }

  void _scheduleSave() {
    _saveTimer?.cancel();
    _saveTimer = Timer(const Duration(milliseconds: 600), _saveProgress);
  }

  void _saveProgress() {
    if (!_scrollController.hasClients) return;
    final maxScroll = _scrollController.position.maxScrollExtent;
    final ratio = maxScroll <= 0
        ? 0.0
        : (_scrollController.offset / maxScroll).clamp(0.0, 1.0);
    _prefs?.setDouble(_progressKey, ratio);

    final auth = context.read<AuthProvider>();
    unawaited(
      context.read<ReadingFeedProvider>().api.saveProgress(
        itemId: widget.item.id,
        mode: _mode,
        scrollRatio: ratio,
        userId: auth.user?.id,
      ),
    );
  }

  String get _progressKey =>
      'freshloop_reader_${widget.item.id}_${_mode == ReadingMode.original ? 'original' : 'compressed'}';

  @override
  Widget build(BuildContext context) {
    final body = _content?.textForMode(_mode) ?? '';
    return Scaffold(
      backgroundColor: AppTheme.darkBackground,
      appBar: AppBar(
        backgroundColor: AppTheme.darkBackground,
        foregroundColor: Colors.white,
        title: Text(
          widget.item.sourceName ?? 'FreshLoop',
          style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w900),
        ),
        actions: [
          if (widget.item.audioUrl != null && widget.item.audioUrl!.isNotEmpty)
            IconButton(
              onPressed: () =>
                  _playSingleItem(_articleToAudioItem(widget.item)),
              icon: const Icon(Icons.headphones),
            ),
        ],
      ),
      body: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 8, 20, 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  _cleanTitle(widget.item.title),
                  style: const TextStyle(
                    color: Colors.white,
                    fontSize: 26,
                    height: 1.12,
                    fontWeight: FontWeight.w900,
                  ),
                ),
                const SizedBox(height: 12),
                Wrap(
                  spacing: 8,
                  runSpacing: 6,
                  children: [
                    _ReaderChip(text: _formatDate(widget.item.publishTime)),
                    _ReaderChip(
                      text: '${widget.item.readingTimeMin ?? 1} min read',
                    ),
                    if (widget.item.qualityScore != null)
                      _ReaderChip(text: '${widget.item.qualityScore}/10'),
                  ],
                ),
                const SizedBox(height: 14),
                Container(
                  padding: const EdgeInsets.all(4),
                  decoration: BoxDecoration(
                    color: Colors.black.withValues(alpha: 0.28),
                    borderRadius: BorderRadius.circular(8),
                    border: Border.all(
                      color: Colors.white.withValues(alpha: 0.08),
                    ),
                  ),
                  child: Row(
                    children: [
                      Expanded(
                        child: _ModeButton(
                          label: '原版',
                          selected: _mode == ReadingMode.original,
                          onTap: () => _switchMode(ReadingMode.original),
                        ),
                      ),
                      Expanded(
                        child: _ModeButton(
                          label: '干货压缩',
                          selected: _mode == ReadingMode.compressed,
                          onTap: () => _switchMode(ReadingMode.compressed),
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
          Expanded(
            child: _loading
                ? const Center(
                    child: CircularProgressIndicator(
                      color: AppTheme.primaryGreen,
                    ),
                  )
                : body.isEmpty
                ? const Center(
                    child: Text(
                      '内容还在生成中',
                      style: TextStyle(color: Colors.white54),
                    ),
                  )
                : ListView(
                    controller: _scrollController,
                    padding: const EdgeInsets.fromLTRB(22, 8, 22, 40),
                    children: _renderMarkdown(body),
                  ),
          ),
        ],
      ),
    );
  }
}

class _MetaChip extends StatelessWidget {
  final String text;

  const _MetaChip({required this.text});

  @override
  Widget build(BuildContext context) {
    return Text(
      text,
      style: const TextStyle(
        color: Colors.white54,
        fontSize: 11,
        fontWeight: FontWeight.w700,
      ),
    );
  }
}

class _ReaderChip extends StatelessWidget {
  final String text;

  const _ReaderChip({required this.text});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 5),
      decoration: BoxDecoration(
        color: AppTheme.surfaceDark,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: Colors.white.withValues(alpha: 0.08)),
      ),
      child: Text(
        text,
        style: const TextStyle(
          color: AppTheme.textMuted,
          fontSize: 11,
          fontWeight: FontWeight.w800,
        ),
      ),
    );
  }
}

class _ModeButton extends StatelessWidget {
  final String label;
  final bool selected;
  final VoidCallback onTap;

  const _ModeButton({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return TextButton(
      onPressed: onTap,
      style: TextButton.styleFrom(
        backgroundColor: selected
            ? AppTheme.surfaceHighlight
            : Colors.transparent,
        foregroundColor: selected ? Colors.white : Colors.white54,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      ),
      child: Text(label, style: const TextStyle(fontWeight: FontWeight.w900)),
    );
  }
}

class _EmptyPanel extends StatelessWidget {
  final IconData icon;
  final String text;
  final String? detail;

  const _EmptyPanel({required this.icon, required this.text, this.detail});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(28),
      decoration: BoxDecoration(
        color: AppTheme.surfaceDark,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: Colors.white.withValues(alpha: 0.06)),
      ),
      child: Column(
        children: [
          Icon(icon, size: 34, color: Colors.white38),
          const SizedBox(height: 12),
          Text(
            text,
            style: const TextStyle(
              color: Colors.white70,
              fontWeight: FontWeight.w800,
            ),
          ),
          if (detail != null) ...[
            const SizedBox(height: 8),
            Text(
              detail!,
              textAlign: TextAlign.center,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(color: Colors.white38, fontSize: 12),
            ),
          ],
        ],
      ),
    );
  }
}

List<Widget> _renderMarkdown(String markdown) {
  final widgets = <Widget>[];
  final bullets = <String>[];
  var orderedBullets = false;
  final paragraph = <String>[];

  void flushBullets() {
    if (bullets.isEmpty) return;
    widgets.add(
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: bullets
              .asMap()
              .entries
              .map(
                (entry) => Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      SizedBox(
                        width: 20,
                        child: Padding(
                          padding: EdgeInsets.only(
                            top: orderedBullets ? 0 : 10,
                          ),
                          child: orderedBullets
                              ? Text(
                                  '${entry.key + 1}.',
                                  style: const TextStyle(
                                    color: AppTheme.primaryGreen,
                                    fontSize: 14,
                                    fontWeight: FontWeight.w900,
                                  ),
                                )
                              : const Icon(
                                  Icons.circle,
                                  size: 6,
                                  color: AppTheme.primaryGreen,
                                ),
                        ),
                      ),
                      const SizedBox(width: 6),
                      Expanded(
                        child: SelectableText(
                          _cleanMarkdownInline(entry.value),
                          style: const TextStyle(
                            color: Colors.white70,
                            height: 1.62,
                            fontSize: 16,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              )
              .toList(),
        ),
      ),
    );
    bullets.clear();
  }

  void flushParagraph() {
    if (paragraph.isEmpty) return;
    final text = paragraph.join('\n').trim();
    paragraph.clear();
    if (text.isEmpty) return;
    widgets.add(_readerParagraph(text));
  }

  void flushAll() {
    flushParagraph();
    flushBullets();
  }

  for (final rawLine in markdown.split('\n')) {
    final line = rawLine.trim();
    if (line.isEmpty) {
      flushAll();
      continue;
    }
    final unordered = RegExp(r'^[-*+]\s+(.+)$').firstMatch(line);
    if (unordered != null) {
      flushParagraph();
      if (bullets.isNotEmpty && orderedBullets) flushBullets();
      orderedBullets = false;
      bullets.add(unordered.group(1)!);
      continue;
    }
    final ordered = RegExp(r'^\d+[.)]\s+(.+)$').firstMatch(line);
    if (ordered != null) {
      flushParagraph();
      if (bullets.isNotEmpty && !orderedBullets) flushBullets();
      orderedBullets = true;
      bullets.add(ordered.group(1)!);
      continue;
    }
    if (line.startsWith('### ')) {
      flushAll();
      widgets.add(_readerHeading(line.substring(4), 19));
      continue;
    }
    if (line.startsWith('## ')) {
      flushAll();
      widgets.add(_readerHeading(line.substring(3), 21));
      continue;
    }
    if (line.startsWith('# ')) {
      flushAll();
      widgets.add(_readerHeading(line.substring(2), 24));
      continue;
    }
    if (line.startsWith('> ')) {
      flushAll();
      widgets.add(_readerQuote(line.substring(2)));
      continue;
    }
    flushBullets();
    paragraph.add(line);
  }
  flushAll();
  return widgets;
}

Widget _readerHeading(String text, double fontSize) {
  return Padding(
    padding: const EdgeInsets.only(top: 22, bottom: 8),
    child: SelectableText(
      _cleanMarkdownInline(text),
      style: TextStyle(
        color: Colors.white,
        fontSize: fontSize,
        height: 1.18,
        fontWeight: FontWeight.w900,
      ),
    ),
  );
}

Widget _readerParagraph(String text) {
  return Padding(
    padding: const EdgeInsets.only(bottom: 15),
    child: SelectableText(
      _cleanMarkdownInline(text),
      style: const TextStyle(color: Colors.white70, fontSize: 17, height: 1.72),
    ),
  );
}

Widget _readerQuote(String text) {
  return Container(
    margin: const EdgeInsets.symmetric(vertical: 12),
    padding: const EdgeInsets.fromLTRB(14, 10, 0, 10),
    decoration: const BoxDecoration(
      border: Border(left: BorderSide(color: AppTheme.primaryGreen, width: 2)),
    ),
    child: SelectableText(
      _cleanMarkdownInline(text),
      style: const TextStyle(color: Colors.white60, fontSize: 16, height: 1.62),
    ),
  );
}

String _cleanMarkdownInline(String text) {
  return text
      .replaceAllMapped(
        RegExp(r'\[([^\]]+)\]\([^)]+\)'),
        (match) => match.group(1) ?? '',
      )
      .replaceAllMapped(
        RegExp(r'\*\*([^*]+)\*\*'),
        (match) => match.group(1) ?? '',
      )
      .replaceAllMapped(RegExp(r'`([^`]+)`'), (match) => match.group(1) ?? '')
      .trim();
}

Item _articleToAudioItem(CuratedFeedItem item) {
  return Item(
    id: 'curated:${item.id}',
    title: _cleanTitle(item.title),
    summary: item.subtitle,
    originalUrl: item.originalUrl,
    coverImageUrl: null,
    audioUrl: item.audioUrl,
    publishTime: item.publishTime,
    createdAt: item.publishTime,
    rating: item.qualityScore,
    tags: item.tags,
    isDeleted: false,
    durationSec: item.durationSec,
    status: 'published',
    category: '精选阅读',
  );
}

Item _weeklyToAudioItem(WeeklyDigest weekly) {
  return Item(
    id: 'weekly:${weekly.id}',
    title: weekly.title,
    summary: weekly.audioScript ?? weekly.digestMarkdown,
    originalUrl: null,
    coverImageUrl: null,
    audioUrl: weekly.audioUrl,
    publishTime: weekly.weekEnd,
    createdAt: weekly.weekEnd,
    rating: null,
    tags: weekly.themesJson,
    isDeleted: false,
    durationSec: weekly.durationSec,
    status: 'published',
    category: '周汇总',
  );
}

Future<void> _playSingleItem(Item item) async {
  if (item.audioUrl == null || item.audioUrl!.isEmpty) return;
  await audioHandler.updateQueueWithItems([item]);
  await audioHandler.skipToQueueItem(0);
  await audioHandler.play();
}

String _cleanTitle(String title) =>
    title.replaceFirst(RegExp(r'^【.*?】'), '').trim();

String _formatDate(int? seconds) {
  if (seconds == null || seconds <= 0) return '';
  return DateFormat(
    'MMM d',
  ).format(DateTime.fromMillisecondsSinceEpoch(seconds * 1000));
}

String _duration(int? seconds) {
  if (seconds == null || seconds <= 0) return '音频';
  final mins = seconds ~/ 60;
  final secs = seconds % 60;
  return '$mins:${secs.toString().padLeft(2, '0')}';
}
