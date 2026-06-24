import 'dart:async';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:url_launcher/url_launcher.dart';

import '../app_shell.dart';
import '../audio_handler.dart';
import '../day_playlist.dart';
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
  String? userId;

  ReadingFeedProvider(this.api) {
    refresh();
  }

  Future<void> refresh() async {
    if (isLoading) return;
    isLoading = true;
    error = null;
    notifyListeners();

    try {
      final itemsFuture = api.fetchItems(userId: userId);
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

  List<DayPlaylistGroup<CuratedFeedItem>> get dayGroups =>
      buildDayPlaylists<CuratedFeedItem>(
        items: items,
        idOf: (item) => item.id,
        timestampSecondsOf: (item) => item.publishTime,
        isPlayable: (item) =>
            item.hasAudio && (item.audioUrl?.trim().isNotEmpty ?? false),
        durationSecondsOf: (item) => item.durationSec,
        dayOrder: DayPlaylistSortOrder.descending,
        itemOrder: DayPlaylistSortOrder.descending,
        playbackOrder: DayPlaylistSortOrder.ascending,
      );

  DayPlaylistGroup<CuratedFeedItem>? dayGroupForItem(String itemId) {
    for (final group in dayGroups) {
      if (group.itemIds.contains(itemId)) return group;
    }
    return null;
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
              else ...[
                ...provider.dayGroups.map(
                  (group) => _ReadingDaySection(group: group),
                ),
              ],
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
        padding: const EdgeInsets.fromLTRB(16, 10, 16, 40),
        children: [
          Align(
            alignment: Alignment.topCenter,
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 760),
              child: Container(
                padding: const EdgeInsets.fromLTRB(20, 18, 20, 28),
                decoration: BoxDecoration(
                  color: const Color(0xFF141E18),
                  borderRadius: BorderRadius.circular(24),
                  border: Border.all(
                    color: Colors.white.withValues(alpha: 0.08),
                  ),
                  boxShadow: const [
                    BoxShadow(
                      color: Color(0x44000000),
                      blurRadius: 28,
                      offset: Offset(0, 16),
                    ),
                  ],
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
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
                        const _ReaderChip(text: 'Weekly Brief'),
                        if (hasAudio)
                          _ReaderChip(text: _duration(weekly.durationSec)),
                      ],
                    ),
                    const SizedBox(height: 18),
                    if (body.isEmpty)
                      const Text(
                        '周汇总文稿还在生成中',
                        style: TextStyle(color: Colors.white54, height: 1.6),
                      )
                    else
                      ..._renderMarkdown(
                        body,
                        variant: _ReaderDocumentVariant.weekly,
                      ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _ReadingDaySection extends StatelessWidget {
  final DayPlaylistGroup<CuratedFeedItem> group;

  const _ReadingDaySection({required this.group});

  @override
  Widget build(BuildContext context) {
    final durationMinutes = group.totalDurationSec > 0
        ? (group.totalDurationSec / 60).ceil()
        : 0;

    return Container(
      margin: const EdgeInsets.only(bottom: 14),
      padding: const EdgeInsets.fromLTRB(14, 14, 14, 10),
      decoration: BoxDecoration(
        color: Colors.white.withValues(alpha: 0.03),
        borderRadius: BorderRadius.circular(18),
        border: Border.all(color: Colors.white.withValues(alpha: 0.06)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      group.title,
                      style: const TextStyle(
                        fontSize: 18,
                        fontWeight: FontWeight.w900,
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      '${group.items.length} 篇文章 · ${group.playableCount} 段可播${durationMinutes > 0 ? ' · $durationMinutes min' : ''}',
                      style: const TextStyle(
                        fontSize: 12,
                        color: Colors.white54,
                      ),
                    ),
                  ],
                ),
              ),
              if (group.playableCount > 0)
                TextButton.icon(
                  onPressed: () => _playCuratedDay(group.items),
                  style: TextButton.styleFrom(
                    foregroundColor: Colors.black,
                    backgroundColor: AppTheme.primaryGreen,
                    shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(999),
                    ),
                  ),
                  icon: const Icon(Icons.play_arrow_rounded),
                  label: const Text('播放当天'),
                ),
            ],
          ),
          const SizedBox(height: 6),
          ...group.items.map(
            (item) => _ArticleCard(item: item, dayGroup: group),
          ),
        ],
      ),
    );
  }
}

class _ArticleCard extends StatelessWidget {
  final CuratedFeedItem item;
  final DayPlaylistGroup<CuratedFeedItem>? dayGroup;

  const _ArticleCard({required this.item, this.dayGroup});

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
          Navigator.of(context).push(
            MaterialPageRoute(
              builder: (_) => ReaderPage(item: item, dayGroup: dayGroup),
            ),
          );
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
  final DayPlaylistGroup<CuratedFeedItem>? dayGroup;

  const ReaderPage({super.key, required this.item, this.dayGroup});

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

  Future<void> _openOriginalUrl() async {
    final rawUrl = widget.item.originalUrl?.trim();
    if (rawUrl == null || rawUrl.isEmpty) return;

    final uri = Uri.tryParse(rawUrl);
    if (uri == null) return;

    final launched = await launchUrl(uri, mode: LaunchMode.externalApplication);
    if (!launched && mounted) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(const SnackBar(content: Text('无法打开原文链接')));
    }
  }

  void _quoteToLoop() {
    context.read<ShellProvider>().openLoopWithDraft(
      LoopComposeDraft(
        title: _cleanTitle(widget.item.title),
        references: [
          LoopDraftReference(
            sourceType: 'article',
            sourceId: widget.item.id,
            sourceUrl: widget.item.originalUrl,
            title: _cleanTitle(widget.item.title),
            quoteText: widget.item.subtitle,
          ),
        ],
      ),
    );
    Navigator.of(context).pop();
  }

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
          if (widget.item.originalUrl != null &&
              widget.item.originalUrl!.trim().isNotEmpty)
            IconButton(
              onPressed: _openOriginalUrl,
              icon: const Icon(Icons.open_in_new_rounded),
              tooltip: '打开原文',
            ),
          if (widget.item.audioUrl != null && widget.item.audioUrl!.isNotEmpty)
            IconButton(
              onPressed: () =>
                  _playSingleItem(_articleToAudioItem(widget.item)),
              icon: const Icon(Icons.headphones),
            ),
          IconButton(
            onPressed: _quoteToLoop,
            icon: const Icon(Icons.format_quote),
            tooltip: '写进 Loop',
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
                Wrap(
                  spacing: 10,
                  runSpacing: 10,
                  children: [
                    if (widget.item.audioUrl != null &&
                        widget.item.audioUrl!.isNotEmpty)
                      FilledButton.icon(
                        onPressed: () =>
                            _playSingleItem(_articleToAudioItem(widget.item)),
                        style: FilledButton.styleFrom(
                          backgroundColor: AppTheme.primaryGreen,
                          foregroundColor: Colors.black,
                        ),
                        icon: const Icon(Icons.headphones),
                        label: const Text('收听本文'),
                      ),
                    if ((widget.dayGroup?.playableCount ?? 0) > 1)
                      OutlinedButton.icon(
                        onPressed: () => _playCuratedDay(
                          widget.dayGroup!.items,
                          startItemId: widget.item.id,
                        ),
                        style: OutlinedButton.styleFrom(
                          foregroundColor: Colors.white,
                          side: BorderSide(
                            color: Colors.white.withValues(alpha: 0.14),
                          ),
                        ),
                        icon: const Icon(Icons.queue_music_rounded),
                        label: Text('播放${widget.dayGroup!.shortTitle}'),
                      ),
                  ],
                ),
                if ((widget.item.audioUrl != null &&
                        widget.item.audioUrl!.isNotEmpty) ||
                    (widget.dayGroup?.playableCount ?? 0) > 1)
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
                    padding: const EdgeInsets.fromLTRB(16, 8, 16, 40),
                    children: [
                      Align(
                        alignment: Alignment.topCenter,
                        child: ConstrainedBox(
                          constraints: const BoxConstraints(maxWidth: 760),
                          child: Container(
                            padding: const EdgeInsets.fromLTRB(20, 18, 20, 28),
                            decoration: BoxDecoration(
                              color: _mode == ReadingMode.original
                                  ? const Color(0xFF19231D)
                                  : const Color(0xFF131A16),
                              borderRadius: BorderRadius.circular(24),
                              border: Border.all(
                                color: Colors.white.withValues(alpha: 0.08),
                              ),
                              boxShadow: const [
                                BoxShadow(
                                  color: Color(0x44000000),
                                  blurRadius: 28,
                                  offset: Offset(0, 16),
                                ),
                              ],
                            ),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Wrap(
                                  spacing: 8,
                                  runSpacing: 8,
                                  crossAxisAlignment: WrapCrossAlignment.center,
                                  children: [
                                    _ReaderChip(
                                      text: _mode == ReadingMode.original
                                          ? '原文整理版'
                                          : '干货压缩',
                                    ),
                                    if (widget.item.sourceName != null &&
                                        widget.item.sourceName!
                                            .trim()
                                            .isNotEmpty)
                                      _ReaderChip(
                                        text: widget.item.sourceName!.trim(),
                                      ),
                                    if (widget.item.originalUrl != null &&
                                        widget.item.originalUrl!
                                            .trim()
                                            .isNotEmpty)
                                      OutlinedButton.icon(
                                        onPressed: _openOriginalUrl,
                                        style: OutlinedButton.styleFrom(
                                          foregroundColor: Colors.white70,
                                          side: BorderSide(
                                            color: Colors.white.withValues(
                                              alpha: 0.12,
                                            ),
                                          ),
                                          shape: RoundedRectangleBorder(
                                            borderRadius: BorderRadius.circular(
                                              999,
                                            ),
                                          ),
                                          padding: const EdgeInsets.symmetric(
                                            horizontal: 14,
                                            vertical: 10,
                                          ),
                                        ),
                                        icon: const Icon(
                                          Icons.open_in_new_rounded,
                                          size: 18,
                                        ),
                                        label: const Text(
                                          '原文链接',
                                          style: TextStyle(
                                            fontWeight: FontWeight.w700,
                                          ),
                                        ),
                                      ),
                                  ],
                                ),
                                const SizedBox(height: 18),
                                ..._renderMarkdown(
                                  body,
                                  variant: _mode == ReadingMode.original
                                      ? _ReaderDocumentVariant.original
                                      : _ReaderDocumentVariant.compressed,
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    ],
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

enum _ReaderDocumentVariant { original, compressed, weekly }

enum _MarkdownBlockType {
  heading,
  paragraph,
  list,
  quote,
  code,
  image,
  divider,
}

class _MarkdownBlock {
  final _MarkdownBlockType type;
  final String text;
  final List<String> items;
  final bool ordered;
  final int level;
  final String? secondary;

  const _MarkdownBlock._({
    required this.type,
    this.text = '',
    this.items = const <String>[],
    this.ordered = false,
    this.level = 0,
    this.secondary,
  });

  const _MarkdownBlock.heading(String text, int level)
    : this._(type: _MarkdownBlockType.heading, text: text, level: level);

  const _MarkdownBlock.paragraph(String text)
    : this._(type: _MarkdownBlockType.paragraph, text: text);

  const _MarkdownBlock.list(List<String> items, {required bool ordered})
    : this._(type: _MarkdownBlockType.list, items: items, ordered: ordered);

  const _MarkdownBlock.quote(String text)
    : this._(type: _MarkdownBlockType.quote, text: text);

  const _MarkdownBlock.code(String text, {String? language})
    : this._(type: _MarkdownBlockType.code, text: text, secondary: language);

  const _MarkdownBlock.image(String src, {String alt = ''})
    : this._(type: _MarkdownBlockType.image, text: src, secondary: alt);

  const _MarkdownBlock.divider() : this._(type: _MarkdownBlockType.divider);
}

List<Widget> _renderMarkdown(
  String markdown, {
  required _ReaderDocumentVariant variant,
}) {
  final blocks = _parseMarkdownBlocks(markdown, variant: variant);
  return blocks
      .asMap()
      .entries
      .map((entry) => _buildMarkdownBlock(entry.value, entry.key, variant))
      .toList();
}

List<_MarkdownBlock> _parseMarkdownBlocks(
  String markdown, {
  required _ReaderDocumentVariant variant,
}) {
  final blocks = <_MarkdownBlock>[];
  final lines = _normalizeMarkdownSource(markdown, variant).split('\n');
  final bullets = <String>[];
  final paragraph = <String>[];
  final codeLines = <String>[];
  var orderedBullets = false;
  var inCodeBlock = false;
  var codeLanguage = '';

  void flushBullets() {
    if (bullets.isEmpty) return;
    blocks.add(
      _MarkdownBlock.list(
        bullets
            .map(_normalizeParagraphText)
            .where((item) => item.isNotEmpty)
            .toList(),
        ordered: orderedBullets,
      ),
    );
    bullets.clear();
  }

  void flushParagraph() {
    if (paragraph.isEmpty) return;
    final text = _normalizeParagraphText(paragraph.join(' '));
    paragraph.clear();
    if (text.isEmpty) return;
    for (final chunk in _splitIntoReaderParagraphs(text, variant)) {
      if (chunk.isNotEmpty) {
        blocks.add(_MarkdownBlock.paragraph(chunk));
      }
    }
  }

  void flushAll() {
    flushParagraph();
    flushBullets();
  }

  for (final rawLine in lines) {
    final line = rawLine.trimRight();
    final trimmed = line.trim();

    if (inCodeBlock) {
      if (trimmed.startsWith('```')) {
        blocks.add(
          _MarkdownBlock.code(
            codeLines.join('\n').replaceFirst(RegExp(r'\n+$'), ''),
            language: codeLanguage.isEmpty ? null : codeLanguage,
          ),
        );
        codeLines.clear();
        inCodeBlock = false;
        codeLanguage = '';
      } else {
        codeLines.add(rawLine.replaceAll('\t', '  '));
      }
      continue;
    }

    final codeFence = RegExp(r'^```([\w-]+)?\s*$').firstMatch(trimmed);
    if (codeFence != null) {
      flushAll();
      inCodeBlock = true;
      codeLanguage = codeFence.group(1) ?? '';
      continue;
    }

    if (trimmed.isEmpty) {
      flushAll();
      continue;
    }

    if (RegExp(r'^(-{3,}|_{3,}|\*{3,})$').hasMatch(trimmed)) {
      flushAll();
      blocks.add(const _MarkdownBlock.divider());
      continue;
    }

    final image = _parseMarkdownImage(trimmed) ?? _parseHtmlImageTag(trimmed);
    if (image != null) {
      flushAll();
      blocks.add(_MarkdownBlock.image(image.$1, alt: image.$2));
      continue;
    }

    if (trimmed.startsWith('### ')) {
      flushAll();
      blocks.add(
        _MarkdownBlock.heading(
          _normalizeParagraphText(trimmed.substring(4)),
          3,
        ),
      );
      continue;
    }
    if (trimmed.startsWith('## ')) {
      flushAll();
      blocks.add(
        _MarkdownBlock.heading(
          _normalizeParagraphText(trimmed.substring(3)),
          2,
        ),
      );
      continue;
    }
    if (trimmed.startsWith('# ')) {
      flushAll();
      blocks.add(
        _MarkdownBlock.heading(
          _normalizeParagraphText(trimmed.substring(2)),
          1,
        ),
      );
      continue;
    }
    if (trimmed.startsWith('> ')) {
      flushAll();
      blocks.add(
        _MarkdownBlock.quote(_normalizeParagraphText(trimmed.substring(2))),
      );
      continue;
    }

    final unordered = RegExp(r'^[-*+]\s+(.+)$').firstMatch(trimmed);
    if (unordered != null) {
      flushParagraph();
      if (bullets.isNotEmpty && orderedBullets) flushBullets();
      orderedBullets = false;
      bullets.add(unordered.group(1)!);
      continue;
    }

    final ordered = RegExp(r'^\d+[.)]\s+(.+)$').firstMatch(trimmed);
    if (ordered != null) {
      flushParagraph();
      if (bullets.isNotEmpty && !orderedBullets) flushBullets();
      orderedBullets = true;
      bullets.add(ordered.group(1)!);
      continue;
    }

    flushBullets();
    paragraph.add(trimmed);
  }

  if (inCodeBlock && codeLines.isNotEmpty) {
    blocks.add(
      _MarkdownBlock.code(
        codeLines.join('\n').replaceFirst(RegExp(r'\n+$'), ''),
        language: codeLanguage.isEmpty ? null : codeLanguage,
      ),
    );
  }

  flushAll();
  return blocks
      .where(
        (block) =>
            block.type != _MarkdownBlockType.list || block.items.isNotEmpty,
      )
      .toList();
}

Widget _buildMarkdownBlock(
  _MarkdownBlock block,
  int index,
  _ReaderDocumentVariant variant,
) {
  switch (block.type) {
    case _MarkdownBlockType.heading:
      return _readerHeading(block.text, level: block.level, variant: variant);
    case _MarkdownBlockType.paragraph:
      return _readerParagraph(block.text, variant: variant);
    case _MarkdownBlockType.list:
      return _readerList(block.items, ordered: block.ordered, variant: variant);
    case _MarkdownBlockType.quote:
      return _readerQuote(block.text, variant: variant);
    case _MarkdownBlockType.code:
      return _readerCodeBlock(block.text, language: block.secondary);
    case _MarkdownBlockType.image:
      return _readerImage(block.text, block.secondary ?? '');
    case _MarkdownBlockType.divider:
      return _readerDivider();
  }
}

Widget _readerHeading(
  String text, {
  required int level,
  required _ReaderDocumentVariant variant,
}) {
  final isCompressed = variant != _ReaderDocumentVariant.original;
  final fontSize = switch ((variant, level)) {
    (_ReaderDocumentVariant.original, 1) => 31.0,
    (_ReaderDocumentVariant.original, 2) => 23.0,
    (_ReaderDocumentVariant.original, _) => 17.0,
    (_, 1) => 29.0,
    (_, 2) => 15.0,
    (_, _) => 18.0,
  };

  final child = SelectableText.rich(
    TextSpan(
      style: TextStyle(
        color: level == 3 && isCompressed ? AppTheme.textMuted : Colors.white,
        fontSize: fontSize,
        height: level == 2 && isCompressed ? 1.0 : 1.16,
        fontWeight: FontWeight.w900,
        letterSpacing: level >= 2 && isCompressed ? 0.6 : 0,
      ),
      children: _inlineSpans(
        text,
        TextStyle(
          color: level == 3 && isCompressed ? AppTheme.textMuted : Colors.white,
          fontSize: fontSize,
          height: level == 2 && isCompressed ? 1.0 : 1.16,
          fontWeight: FontWeight.w900,
          letterSpacing: level >= 2 && isCompressed ? 0.6 : 0,
        ),
      ),
    ),
  );

  return Padding(
    padding: EdgeInsets.only(
      top: level == 1 ? 8 : 26,
      bottom: level == 2 ? 12 : 8,
    ),
    child: isCompressed && level == 2
        ? Container(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 7),
            decoration: BoxDecoration(
              color: AppTheme.primaryGreen.withValues(alpha: 0.10),
              borderRadius: BorderRadius.circular(999),
              border: Border.all(
                color: AppTheme.primaryGreen.withValues(alpha: 0.22),
              ),
            ),
            child: child,
          )
        : child,
  );
}

Widget _readerParagraph(
  String text, {
  required _ReaderDocumentVariant variant,
}) {
  final isOriginal = variant == _ReaderDocumentVariant.original;
  final style = TextStyle(
    color: isOriginal ? Colors.white.withValues(alpha: 0.92) : Colors.white70,
    fontSize: isOriginal ? 18 : 16.5,
    height: isOriginal ? 1.92 : 1.82,
    fontFamily: isOriginal ? 'serif' : null,
    letterSpacing: isOriginal ? 0.1 : 0,
  );

  return Padding(
    padding: const EdgeInsets.only(bottom: 16),
    child: SelectableText.rich(
      TextSpan(style: style, children: _inlineSpans(text, style)),
    ),
  );
}

Widget _readerList(
  List<String> items, {
  required bool ordered,
  required _ReaderDocumentVariant variant,
}) {
  final isCompressed = variant != _ReaderDocumentVariant.original;
  return Padding(
    padding: const EdgeInsets.only(bottom: 18),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: items.asMap().entries.map((entry) {
        final style = TextStyle(
          color: Colors.white.withValues(alpha: 0.86),
          fontSize: variant == _ReaderDocumentVariant.original ? 17.5 : 16.2,
          height: variant == _ReaderDocumentVariant.original ? 1.84 : 1.78,
          fontFamily: variant == _ReaderDocumentVariant.original
              ? 'serif'
              : null,
        );
        final itemBody = SelectableText.rich(
          TextSpan(style: style, children: _inlineSpans(entry.value, style)),
        );

        return Container(
          margin: const EdgeInsets.only(bottom: 10),
          padding: isCompressed
              ? const EdgeInsets.symmetric(horizontal: 14, vertical: 13)
              : EdgeInsets.zero,
          decoration: isCompressed
              ? BoxDecoration(
                  color: Colors.white.withValues(alpha: 0.04),
                  borderRadius: BorderRadius.circular(16),
                  border: Border.all(
                    color: Colors.white.withValues(alpha: 0.06),
                  ),
                )
              : null,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: 24,
                child: Padding(
                  padding: EdgeInsets.only(top: ordered ? 0 : 10),
                  child: ordered
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
              const SizedBox(width: 8),
              Expanded(child: itemBody),
            ],
          ),
        );
      }).toList(),
    ),
  );
}

Widget _readerQuote(String text, {required _ReaderDocumentVariant variant}) {
  final style = TextStyle(
    color: Colors.white.withValues(alpha: 0.72),
    fontSize: variant == _ReaderDocumentVariant.original ? 17 : 16,
    height: 1.8,
    fontFamily: variant == _ReaderDocumentVariant.original ? 'serif' : null,
  );

  return Container(
    margin: const EdgeInsets.symmetric(vertical: 10),
    padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
    decoration: BoxDecoration(
      color: Colors.white.withValues(alpha: 0.035),
      borderRadius: BorderRadius.circular(16),
      border: Border(
        left: BorderSide(
          color: AppTheme.primaryGreen.withValues(alpha: 0.78),
          width: 2,
        ),
      ),
    ),
    child: SelectableText.rich(
      TextSpan(style: style, children: _inlineSpans(text, style)),
    ),
  );
}

Widget _readerCodeBlock(String code, {String? language}) {
  return Container(
    margin: const EdgeInsets.symmetric(vertical: 10),
    padding: const EdgeInsets.fromLTRB(14, 12, 14, 14),
    decoration: BoxDecoration(
      color: const Color(0xFF0D1410),
      borderRadius: BorderRadius.circular(18),
      border: Border.all(color: AppTheme.primaryGreen.withValues(alpha: 0.16)),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (language != null && language.trim().isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(
              language.toUpperCase(),
              style: const TextStyle(
                color: AppTheme.textMuted,
                fontSize: 11,
                fontWeight: FontWeight.w800,
                letterSpacing: 0.8,
              ),
            ),
          ),
        SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: SelectableText(
            code,
            style: const TextStyle(
              color: Color(0xFFD7FCE4),
              fontSize: 14.2,
              height: 1.72,
              fontFamily: 'monospace',
            ),
          ),
        ),
      ],
    ),
  );
}

Widget _readerImage(String src, String alt) {
  return Padding(
    padding: const EdgeInsets.symmetric(vertical: 10),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        ClipRRect(
          borderRadius: BorderRadius.circular(18),
          child: Image.network(
            src,
            fit: BoxFit.cover,
            errorBuilder: (context, error, stackTrace) {
              return Container(
                height: 180,
                alignment: Alignment.center,
                color: Colors.white.withValues(alpha: 0.04),
                child: const Text(
                  '图片暂时无法加载',
                  style: TextStyle(color: Colors.white38),
                ),
              );
            },
          ),
        ),
        if (alt.trim().isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 10),
            child: Text(
              alt.trim(),
              style: const TextStyle(
                color: Colors.white38,
                fontSize: 12.5,
                height: 1.5,
              ),
            ),
          ),
      ],
    ),
  );
}

Widget _readerDivider() {
  return Container(
    margin: const EdgeInsets.symmetric(vertical: 18),
    height: 1,
    width: 140,
    decoration: const BoxDecoration(
      gradient: LinearGradient(
        colors: [Colors.transparent, AppTheme.primaryGreen, Colors.transparent],
      ),
    ),
  );
}

List<InlineSpan> _inlineSpans(String text, TextStyle baseStyle) {
  final spans = <InlineSpan>[];
  final pattern = RegExp(r'(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))');
  var cursor = 0;

  for (final match in pattern.allMatches(text)) {
    if (match.start > cursor) {
      spans.add(TextSpan(text: text.substring(cursor, match.start)));
    }
    final token = match.group(0)!;
    if (token.startsWith('**')) {
      spans.add(
        TextSpan(
          text: token.substring(2, token.length - 2),
          style: baseStyle.copyWith(
            fontWeight: FontWeight.w900,
            color: Colors.white,
            fontFamily: null,
          ),
        ),
      );
    } else if (token.startsWith('`')) {
      spans.add(
        WidgetSpan(
          alignment: PlaceholderAlignment.middle,
          child: Container(
            margin: const EdgeInsets.symmetric(horizontal: 1),
            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
            decoration: BoxDecoration(
              color: Colors.white.withValues(alpha: 0.08),
              borderRadius: BorderRadius.circular(6),
            ),
            child: Text(
              token.substring(1, token.length - 1),
              style: baseStyle.copyWith(
                fontSize: (baseStyle.fontSize ?? 16) * 0.9,
                fontFamily: 'monospace',
                color: const Color(0xFFC5F7D7),
              ),
            ),
          ),
        ),
      );
    } else {
      final link = RegExp(r'^\[([^\]]+)\]\(([^)]+)\)$').firstMatch(token);
      spans.add(
        TextSpan(
          text: link?.group(1) ?? token,
          style: baseStyle.copyWith(
            color: AppTheme.primaryGreen,
            decoration: TextDecoration.underline,
            decorationColor: AppTheme.primaryGreen.withValues(alpha: 0.42),
          ),
        ),
      );
    }
    cursor = match.end;
  }

  if (cursor < text.length) {
    spans.add(TextSpan(text: text.substring(cursor)));
  }

  return spans;
}

String _normalizeMarkdownSource(
  String markdown,
  _ReaderDocumentVariant variant,
) {
  var normalized = markdown.replaceAll('\r\n', '\n').trim();
  normalized = normalized.replaceAll(
    RegExp(r'<br\s*/?>', caseSensitive: false),
    '\n',
  );
  if (variant == _ReaderDocumentVariant.original &&
      _looksLikeCollapsedOriginal(normalized)) {
    normalized = _splitIntoReaderParagraphs(normalized, variant).join('\n\n');
  }
  return normalized;
}

bool _looksLikeCollapsedOriginal(String text) {
  final trimmed = text.trim();
  if (trimmed.length < 900) return false;
  if (trimmed.contains('\n\n')) return false;
  return !RegExp(
    r'(^|\n)\s*(#{1,3}\s|[-*+]\s|\d+[.)]\s|>\s|```|!\[|<img\b)',
    multiLine: true,
  ).hasMatch(trimmed);
}

List<String> _splitIntoReaderParagraphs(
  String text,
  _ReaderDocumentVariant variant,
) {
  final normalized = _normalizeParagraphText(text);
  if (variant != _ReaderDocumentVariant.original || normalized.length < 460) {
    return [normalized];
  }

  final sentences = _splitSentences(normalized);
  if (sentences.length < 3) {
    return [normalized];
  }

  final chunks = <String>[];
  final buffer = <String>[];
  var charCount = 0;

  for (final sentence in sentences) {
    buffer.add(sentence);
    charCount += sentence.length;
    if (charCount >= 260 || buffer.length >= 3) {
      chunks.add(_normalizeParagraphText(buffer.join(' ')));
      buffer.clear();
      charCount = 0;
    }
  }

  if (buffer.isNotEmpty) {
    chunks.add(_normalizeParagraphText(buffer.join(' ')));
  }

  return chunks.where((item) => item.isNotEmpty).toList();
}

List<String> _splitSentences(String text) {
  final sentences = <String>[];
  var current = '';

  for (var i = 0; i < text.length; i++) {
    final char = text[i];
    current += char;

    final next = _nextSignificantChar(text, i + 1);
    final boundary =
        char == '。' ||
        char == '！' ||
        char == '？' ||
        char == '!' ||
        char == '?' ||
        (char == '.' && next != null && RegExp("[A-Z0-9\"']").hasMatch(next));

    if (boundary) {
      final sentence = current.trim();
      if (sentence.isNotEmpty) {
        sentences.add(sentence);
      }
      current = '';
    }
  }

  if (current.trim().isNotEmpty) {
    sentences.add(current.trim());
  }

  return sentences;
}

String? _nextSignificantChar(String text, int start) {
  for (var i = start; i < text.length; i++) {
    final char = text[i];
    if (char.trim().isNotEmpty) {
      return char;
    }
  }
  return null;
}

(String, String)? _parseHtmlImageTag(String line) {
  final tagMatch = RegExp(
    r'<img\b[^>]*>',
    caseSensitive: false,
  ).firstMatch(line);
  if (tagMatch == null) return null;
  final tag = tagMatch.group(0)!;
  final src = RegExp(
    "\\bsrc=[\"']([^\"']+)[\"']",
    caseSensitive: false,
  ).firstMatch(tag)?.group(1);
  if (src == null || src.isEmpty) return null;
  final alt =
      RegExp(
        "\\balt=[\"']([^\"']*)[\"']",
        caseSensitive: false,
      ).firstMatch(tag)?.group(1) ??
      '';
  return (src, _decodeHtmlEntities(alt));
}

(String, String)? _parseMarkdownImage(String line) {
  final match = RegExp(
    r'^!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)$',
  ).firstMatch(line.trim());
  if (match == null) return null;
  return (match.group(2)!, _decodeHtmlEntities(match.group(1) ?? ''));
}

String _decodeHtmlEntities(String text) {
  return text
      .replaceAll('&nbsp;', ' ')
      .replaceAll('&amp;', '&')
      .replaceAll('&lt;', '<')
      .replaceAll('&gt;', '>')
      .replaceAll('&quot;', '"')
      .replaceAll('&#39;', "'");
}

String _stripUnsupportedHtml(String text) {
  return _decodeHtmlEntities(
    text
        .replaceAll(RegExp(r'<br\s*/?>', caseSensitive: false), '\n')
        .replaceAll(RegExp(r'</p>', caseSensitive: false), '\n\n')
        .replaceAll(RegExp(r'<[^>]+>'), ' '),
  );
}

String normalizeReaderParagraphText(String text) {
  return _removeOrphanDollarMarkers(_stripUnsupportedHtml(text))
      .replaceAll('\u00a0', ' ')
      .replaceAll(RegExp(r'[ \t]+'), ' ')
      .replaceAllMapped(
        RegExp(r'\s+([,.;:!?])'),
        (match) => match.group(1) ?? '',
      )
      .replaceAllMapped(
        RegExp(r'\s*([，。！？：；、])'),
        (match) => match.group(1) ?? '',
      )
      .replaceAllMapped(
        RegExp(r'([（【《“‘])\s+'),
        (match) => match.group(1) ?? '',
      )
      .replaceAllMapped(
        RegExp(r'\s+([）】》”’])'),
        (match) => match.group(1) ?? '',
      )
      .replaceAllMapped(
        RegExp(r'([\u3400-\u9fff])\s+([\u3400-\u9fff])'),
        (match) => '${match.group(1)}${match.group(2)}',
      )
      .trim();
}

String _removeOrphanDollarMarkers(String text) {
  final buffer = StringBuffer();
  var index = 0;

  while (index < text.length) {
    final current = text[index];
    if (current == r'$') {
      var end = index + 1;
      var digitCount = 0;
      while (end < text.length && digitCount < 2 && _isAsciiDigit(text[end])) {
        end += 1;
        digitCount += 1;
      }

      if (digitCount > 0) {
        final next = end < text.length ? text[end] : null;
        final nextIsMoney =
            next != null && (_isAsciiDigit(next) || next == ',' || next == '.');
        if (!nextIsMoney) {
          final currentOutput = buffer.toString();
          final previous = currentOutput.isEmpty
              ? null
              : currentOutput[currentOutput.length - 1];
          final previousIsInline =
              previous != null &&
              previous.trim().isNotEmpty &&
              previous != r'$';
          final previousIsBoundary =
              previous == null || (!_isWordChar(previous) && previous != r'$');
          var nextSignificant = end;
          while (nextSignificant < text.length &&
              text[nextSignificant].trim().isEmpty) {
            nextSignificant += 1;
          }
          final followedByTerminal =
              nextSignificant >= text.length ||
              _isTerminalAfterDollarMarker(text[nextSignificant]);

          if (previousIsInline || (previousIsBoundary && followedByTerminal)) {
            index = end;
            continue;
          }
        }
      }
    }

    buffer.write(current);
    index += 1;
  }

  return buffer.toString().replaceAll(RegExp(r'[ \t]{2,}'), ' ');
}

bool _isAsciiDigit(String value) =>
    value.length == 1 && value.codeUnitAt(0) >= 48 && value.codeUnitAt(0) <= 57;

bool _isWordChar(String value) {
  if (value.length != 1) return false;
  final code = value.codeUnitAt(0);
  return (code >= 48 && code <= 57) ||
      (code >= 65 && code <= 90) ||
      (code >= 97 && code <= 122) ||
      value == '_';
}

bool _isTerminalAfterDollarMarker(String value) =>
    value == ')' ||
    value == ']' ||
    value == '}' ||
    value == '。' ||
    value == '！' ||
    value == '？' ||
    value == '；' ||
    value == '，' ||
    value == '、' ||
    value == ',' ||
    value == '.' ||
    value == ';' ||
    value == ':' ||
    value == '!' ||
    value == '?';

String _normalizeParagraphText(String text) =>
    normalizeReaderParagraphText(text);

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
  await audioHandler.updateQueueWithItems([
    item,
  ], playbackMode: QueuePlaybackMode.staticPlaylist);
  await audioHandler.skipToQueueItem(0);
  await audioHandler.play();
}

Future<void> _playCuratedDay(
  List<CuratedFeedItem> items, {
  String? startItemId,
}) async {
  final queueItems =
      items
          .where(
            (item) =>
                item.hasAudio && (item.audioUrl?.trim().isNotEmpty ?? false),
          )
          .toList()
        ..sort((left, right) {
          final leftTs = left.publishTime ?? 0;
          final rightTs = right.publishTime ?? 0;
          final byTime = leftTs.compareTo(rightTs);
          if (byTime != 0) return byTime;
          return left.id.compareTo(right.id);
        });
  if (queueItems.isEmpty) return;

  final audioItems = queueItems
      .map(_articleToAudioItem)
      .toList(growable: false);
  final startIndex = startItemId == null
      ? 0
      : queueItems.indexWhere((item) => item.id == startItemId);
  final safeIndex = startIndex >= 0 ? startIndex : 0;

  await audioHandler.updateQueueWithItems(
    audioItems,
    playbackMode: QueuePlaybackMode.staticPlaylist,
  );
  await audioHandler.skipToQueueItem(safeIndex);
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
