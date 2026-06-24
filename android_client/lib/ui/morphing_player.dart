import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:audio_service/audio_service.dart';
import 'package:audio_video_progress_bar/audio_video_progress_bar.dart';
import 'package:http/http.dart' as http;
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:url_launcher/url_launcher.dart';
import '../main.dart'; // To access audioHandler and FeedProvider
import 'theme.dart';
import 'animated_eq.dart';

class _TranscriptSourceLink {
  final String url;
  final String title;
  final String summary;

  const _TranscriptSourceLink({
    required this.url,
    required this.title,
    required this.summary,
  });

  factory _TranscriptSourceLink.fromJson(Map<String, dynamic> json) {
    return _TranscriptSourceLink(
      url: json['source_url'] as String? ?? '',
      title: (json['source_title'] as String?)?.trim().isNotEmpty == true
          ? (json['source_title'] as String).trim()
          : '原始内容',
      summary: (json['source_summary'] as String?)?.trim() ?? '',
    );
  }
}

class MorphingPlayer extends StatefulWidget {
  const MorphingPlayer({super.key});

  @override
  State<MorphingPlayer> createState() => _MorphingPlayerState();
}

class _MorphingPlayerState extends State<MorphingPlayer> {
  bool _isExpanded = false;
  String _panelView = 'transcript';
  double _playbackSpeed = 1.0;
  StreamSubscription<MediaItem?>? _mediaItemSubscription;
  List<_TranscriptSourceLink> _sourceLinks = const [];
  bool _isLoadingSources = false;
  String? _sourceError;
  String? _loadedSourceItemId;

  @override
  void initState() {
    super.initState();
    _loadSpeed();
    _mediaItemSubscription = audioHandler.mediaItem.listen(
      _handleMediaItemChanged,
    );
  }

  @override
  void dispose() {
    _mediaItemSubscription?.cancel();
    super.dispose();
  }

  Future<void> _loadSpeed() async {
    final prefs = await SharedPreferences.getInstance();
    final saved = prefs.getDouble('freshloop_playback_speed');
    if (saved != null && [1.0, 1.2, 1.5, 2.0].contains(saved)) {
      setState(() => _playbackSpeed = saved);
      audioHandler.setSpeed(saved);
    }
  }

  void _toggleExpand() {
    setState(() => _isExpanded = !_isExpanded);
  }

  void _cycleSpeed() async {
    setState(() {
      if (_playbackSpeed == 1.0) {
        _playbackSpeed = 1.2;
      } else if (_playbackSpeed == 1.2) {
        _playbackSpeed = 1.5;
      } else if (_playbackSpeed == 1.5) {
        _playbackSpeed = 2.0;
      } else {
        _playbackSpeed = 1.0;
      }
    });
    audioHandler.setSpeed(_playbackSpeed);
    final prefs = await SharedPreferences.getInstance();
    await prefs.setDouble('freshloop_playback_speed', _playbackSpeed);
  }

  Future<void> _handleMediaItemChanged(MediaItem? mediaItem) async {
    final itemId = mediaItem?.id;
    if (itemId == null || itemId.isEmpty) {
      if (!mounted) return;
      setState(() {
        _loadedSourceItemId = null;
        _sourceLinks = const [];
        _sourceError = null;
        _isLoadingSources = false;
      });
      return;
    }

    if (_loadedSourceItemId == itemId) {
      return;
    }

    if (mounted) {
      setState(() {
        _loadedSourceItemId = itemId;
        _sourceLinks = const [];
        _sourceError = null;
        _isLoadingSources = true;
      });
    }

    try {
      final uri = Uri.parse(
        '$baseUrl/api/items/${Uri.encodeComponent(itemId)}/sources',
      );
      final response = await http.get(uri);
      if (response.statusCode < 200 || response.statusCode >= 300) {
        throw Exception('HTTP ${response.statusCode}');
      }

      final decoded = jsonDecode(response.body);
      final links = decoded is List
          ? decoded
                .whereType<Map<String, dynamic>>()
                .map(_TranscriptSourceLink.fromJson)
                .where((link) => link.url.trim().isNotEmpty)
                .toList()
          : <_TranscriptSourceLink>[];

      if (!mounted || _loadedSourceItemId != itemId) return;
      setState(() {
        _sourceLinks = links;
        _isLoadingSources = false;
        _sourceError = null;
      });
    } catch (error) {
      if (!mounted || _loadedSourceItemId != itemId) return;
      setState(() {
        _sourceLinks = const [];
        _isLoadingSources = false;
        _sourceError = error.toString();
      });
    }
  }

  Future<void> _openExternalLink(String rawUrl) async {
    final uri = Uri.tryParse(rawUrl.trim());
    if (uri == null) {
      if (!mounted) return;
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(const SnackBar(content: Text('链接格式无效')));
      return;
    }

    final launched = await launchUrl(uri, mode: LaunchMode.externalApplication);
    if (!launched && mounted) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(const SnackBar(content: Text('无法打开链接')));
    }
  }

  Widget _buildTranscriptPanel(MediaItem mediaItem) {
    final summary = (mediaItem.extras?['summary'] as String?)?.trim();
    final originalUrl = (mediaItem.extras?['original_url'] as String?)?.trim();
    final hasSummary = summary != null && summary.isNotEmpty;
    final hasOriginalUrl = originalUrl != null && originalUrl.isNotEmpty;
    final hasSources = _sourceLinks.isNotEmpty;

    return SingleChildScrollView(
      padding: const EdgeInsets.all(20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            hasSummary ? summary : '暂无文稿内容',
            style: const TextStyle(
              fontSize: 16,
              color: Colors.white70,
              height: 1.6,
            ),
          ),
          if (hasOriginalUrl ||
              hasSources ||
              _isLoadingSources ||
              _sourceError != null) ...[
            const SizedBox(height: 22),
            const Text(
              '原始内容链接',
              style: TextStyle(
                color: AppTheme.primaryGreen,
                fontSize: 13,
                fontWeight: FontWeight.w900,
              ),
            ),
            const SizedBox(height: 10),
            if (hasOriginalUrl)
              _SourceLinkCard(
                title: '打开原文',
                summary: originalUrl,
                buttonLabel: '打开链接',
                onTap: () => _openExternalLink(originalUrl),
              ),
            if (_isLoadingSources)
              const Padding(
                padding: EdgeInsets.only(top: 8),
                child: SizedBox(
                  width: 20,
                  height: 20,
                  child: CircularProgressIndicator(
                    strokeWidth: 2,
                    color: AppTheme.primaryGreen,
                  ),
                ),
              )
            else if (hasSources) ...[
              if (hasOriginalUrl) const SizedBox(height: 10),
              ..._sourceLinks.map(
                (source) => Padding(
                  padding: const EdgeInsets.only(bottom: 10),
                  child: _SourceLinkCard(
                    title: source.title,
                    summary: source.summary.isNotEmpty
                        ? source.summary
                        : source.url,
                    buttonLabel: '查看来源',
                    onTap: () => _openExternalLink(source.url),
                  ),
                ),
              ),
            ] else if (_sourceError != null && !hasOriginalUrl)
              Text(
                '来源加载失败：$_sourceError',
                style: const TextStyle(color: Colors.white38, fontSize: 12),
              ),
          ],
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return StreamBuilder<MediaItem?>(
      stream: audioHandler.mediaItem,
      builder: (context, snapshot) {
        final mediaItem = snapshot.data;
        if (mediaItem == null) return const SizedBox.shrink();

        return AnimatedPositioned(
          duration: const Duration(milliseconds: 400),
          curve: Curves.fastOutSlowIn,
          bottom: _isExpanded ? 0 : 16,
          left: _isExpanded ? 0 : 16,
          right: _isExpanded ? 0 : 16,
          top: _isExpanded ? 50 : null,
          height: _isExpanded ? null : 72,
          child: GestureDetector(
            onTap: _isExpanded ? null : _toggleExpand,
            child: AnimatedContainer(
              duration: const Duration(milliseconds: 400),
              curve: Curves.fastOutSlowIn,
              decoration: BoxDecoration(
                color: _isExpanded
                    ? AppTheme.surfaceDark
                    : const Color(0xFF1E1E1E),
                borderRadius: BorderRadius.circular(_isExpanded ? 32 : 36),
                boxShadow: [
                  BoxShadow(
                    color: Colors.black.withValues(alpha: 0.5),
                    blurRadius: 16,
                    offset: const Offset(0, 8),
                  ),
                ],
                border: Border.all(
                  color: Colors.white.withValues(alpha: 0.1),
                  width: 1,
                ),
              ),
              child: ClipRRect(
                borderRadius: BorderRadius.circular(_isExpanded ? 32 : 36),
                child: _isExpanded
                    ? _buildExpandedPlayer(mediaItem)
                    : _buildCollapsedPlayer(mediaItem),
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _buildCollapsedPlayer(MediaItem mediaItem) {
    return StreamBuilder<PlaybackState>(
      stream: audioHandler.playbackState,
      builder: (context, stateSnapshot) {
        final state = stateSnapshot.data;
        final playing = state?.playing ?? false;
        final isBuffering =
            state?.processingState == AudioProcessingState.buffering;

        return Row(
          children: [
            const SizedBox(width: 8),
            // Circular Art / EQ
            Container(
              width: 56,
              height: 56,
              decoration: const BoxDecoration(
                shape: BoxShape.circle,
                color: AppTheme.surfaceHighlight,
              ),
              child: playing
                  ? const Center(child: AnimatedEqualizer(size: 'sm'))
                  : const Icon(Icons.graphic_eq, color: Colors.white54),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    mediaItem.title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontWeight: FontWeight.bold,
                      fontSize: 14,
                    ),
                  ),
                  Text(
                    mediaItem.artist ?? 'FreshLoop',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      color: AppTheme.textMuted,
                      fontSize: 12,
                    ),
                  ),
                ],
              ),
            ),
            if (isBuffering)
              const Padding(
                padding: EdgeInsets.all(12.0),
                child: SizedBox(
                  width: 24,
                  height: 24,
                  child: CircularProgressIndicator(
                    strokeWidth: 2,
                    color: AppTheme.primaryGreen,
                  ),
                ),
              )
            else
              IconButton(
                icon: Icon(
                  playing
                      ? Icons.pause_circle_filled
                      : Icons.play_circle_filled,
                  size: 40,
                  color: Colors.white,
                ),
                onPressed: () =>
                    playing ? audioHandler.pause() : audioHandler.play(),
              ),
            IconButton(
              icon: const Icon(
                Icons.skip_next,
                size: 32,
                color: Colors.white54,
              ),
              onPressed: audioHandler.skipToNext,
            ),
            const SizedBox(width: 8),
          ],
        );
      },
    );
  }

  Widget _buildExpandedPlayer(MediaItem mediaItem) {
    return Scaffold(
      backgroundColor: Colors.transparent,
      appBar: AppBar(
        leading: IconButton(
          icon: const Icon(Icons.keyboard_arrow_down, size: 32),
          onPressed: _toggleExpand,
        ),
        actions: [
          TextButton(
            onPressed: _cycleSpeed,
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
              decoration: BoxDecoration(
                color: AppTheme.primaryGreen.withValues(alpha: 0.15),
                borderRadius: BorderRadius.circular(16),
                border: Border.all(
                  color: AppTheme.primaryGreen.withValues(alpha: 0.3),
                ),
              ),
              child: Text(
                '${_playbackSpeed}x',
                style: const TextStyle(
                  color: AppTheme.primaryGreen,
                  fontWeight: FontWeight.bold,
                  fontSize: 14,
                ),
              ),
            ),
          ),
          const SizedBox(width: 12),
        ],
      ),
      body: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16.0),
        child: Column(
          children: [
            const SizedBox(height: 12),
            // Title Area
            Text(
              mediaItem.title,
              textAlign: TextAlign.center,
              style: const TextStyle(
                fontSize: 18,
                fontWeight: FontWeight.bold,
                height: 1.2,
              ),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: 4),
            Text(
              mediaItem.artist ?? 'FreshLoop',
              style: const TextStyle(color: Colors.white54, fontSize: 14),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: 20),

            // Progress Bar
            StreamBuilder<Duration>(
              stream: AudioService.position,
              builder: (context, posSnapshot) {
                final position = posSnapshot.data ?? Duration.zero;
                final duration = mediaItem.duration ?? Duration.zero;
                return ProgressBar(
                  progress: position,
                  total: duration,
                  progressBarColor: AppTheme.primaryGreen,
                  baseBarColor: Colors.white12,
                  thumbColor: AppTheme.primaryGreen,
                  timeLabelTextStyle: const TextStyle(
                    color: Colors.white54,
                    fontSize: 12,
                    fontFamily: 'monospace',
                  ),
                  onSeek: (duration) {
                    audioHandler.seek(duration);
                  },
                );
              },
            ),
            const SizedBox(height: 16),

            // Controls
            StreamBuilder<PlaybackState>(
              stream: audioHandler.playbackState,
              builder: (context, stateSnapshot) {
                final playing = stateSnapshot.data?.playing ?? false;
                return Row(
                  mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                  children: [
                    IconButton(
                      icon: const Icon(
                        Icons.replay_10,
                        color: Colors.white54,
                        size: 28,
                      ),
                      onPressed: () => audioHandler.rewind(),
                    ),
                    IconButton(
                      icon: const Icon(
                        Icons.skip_previous,
                        color: Colors.white,
                        size: 36,
                      ),
                      onPressed: audioHandler.skipToPrevious,
                    ),
                    IconButton(
                      icon: Icon(
                        playing
                            ? Icons.pause_circle_filled
                            : Icons.play_circle_filled,
                        size: 64,
                        color: AppTheme.primaryGreen,
                      ),
                      padding: EdgeInsets.zero,
                      onPressed: () =>
                          playing ? audioHandler.pause() : audioHandler.play(),
                    ),
                    IconButton(
                      icon: const Icon(
                        Icons.skip_next,
                        color: Colors.white,
                        size: 36,
                      ),
                      onPressed: audioHandler.skipToNext,
                    ),
                    IconButton(
                      icon: const Icon(
                        Icons.forward_30,
                        color: Colors.white54,
                        size: 28,
                      ),
                      onPressed: () => audioHandler.fastForward(),
                    ),
                  ],
                );
              },
            ),
            const SizedBox(height: 16),

            // Segmented Toggle
            Container(
              decoration: BoxDecoration(
                color: Colors.black26,
                borderRadius: BorderRadius.circular(16),
              ),
              padding: const EdgeInsets.all(4),
              child: Row(
                children: [
                  Expanded(
                    child: GestureDetector(
                      onTap: () => setState(() => _panelView = 'transcript'),
                      child: Container(
                        padding: const EdgeInsets.symmetric(vertical: 10),
                        decoration: BoxDecoration(
                          color: _panelView == 'transcript'
                              ? AppTheme.surfaceHighlight
                              : Colors.transparent,
                          borderRadius: BorderRadius.circular(12),
                        ),
                        child: Row(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            Icon(
                              Icons.article_outlined,
                              size: 16,
                              color: _panelView == 'transcript'
                                  ? Colors.white
                                  : Colors.white54,
                            ),
                            const SizedBox(width: 6),
                            Text(
                              '文稿',
                              style: TextStyle(
                                color: _panelView == 'transcript'
                                    ? Colors.white
                                    : Colors.white54,
                                fontWeight: FontWeight.bold,
                                fontSize: 14,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                  Expanded(
                    child: GestureDetector(
                      onTap: () => setState(() => _panelView = 'playlist'),
                      child: Container(
                        padding: const EdgeInsets.symmetric(vertical: 10),
                        decoration: BoxDecoration(
                          color: _panelView == 'playlist'
                              ? AppTheme.surfaceHighlight
                              : Colors.transparent,
                          borderRadius: BorderRadius.circular(12),
                        ),
                        child: Row(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            Icon(
                              Icons.queue_music,
                              size: 16,
                              color: _panelView == 'playlist'
                                  ? Colors.white
                                  : Colors.white54,
                            ),
                            const SizedBox(width: 6),
                            Text(
                              '列表',
                              style: TextStyle(
                                color: _panelView == 'playlist'
                                    ? Colors.white
                                    : Colors.white54,
                                fontWeight: FontWeight.bold,
                                fontSize: 14,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 8),

            // Panel Content
            Expanded(
              child: Container(
                width: double.infinity,
                clipBehavior: Clip.antiAlias,
                decoration: BoxDecoration(
                  color: Colors.black26,
                  borderRadius: BorderRadius.circular(20),
                  border: Border.all(color: Colors.white10),
                ),
                child: _panelView == 'transcript'
                    ? _buildTranscriptPanel(mediaItem)
                    : StreamBuilder<List<MediaItem>>(
                        stream: audioHandler.queue,
                        builder: (context, queueSnapshot) {
                          final queueItems =
                              queueSnapshot.data ?? const <MediaItem>[];
                          if (queueItems.isEmpty) {
                            return const Center(
                              child: Column(
                                mainAxisAlignment: MainAxisAlignment.center,
                                children: [
                                  Icon(
                                    Icons.queue_music,
                                    color: Colors.white24,
                                    size: 48,
                                  ),
                                  SizedBox(height: 12),
                                  Text(
                                    '队列为空',
                                    style: TextStyle(
                                      color: Colors.white38,
                                      fontSize: 16,
                                    ),
                                  ),
                                ],
                              ),
                            );
                          }
                          return ListView.separated(
                            padding: const EdgeInsets.symmetric(
                              vertical: 8,
                              horizontal: 8,
                            ),
                            separatorBuilder: (context, index) =>
                                const SizedBox(height: 2),
                            itemCount: queueItems.length,
                            itemBuilder: (context, index) {
                              final item = queueItems[index];
                              final isPlaying = item.id == mediaItem.id;
                              final duration = item.duration;
                              final isRadioItem = !item.id.contains(':');
                              final queueTile = Material(
                                color: Colors.transparent,
                                child: InkWell(
                                  borderRadius: BorderRadius.circular(10),
                                  onTap: () =>
                                      audioHandler.skipToQueueItem(index),
                                  child: Container(
                                    padding: const EdgeInsets.symmetric(
                                      horizontal: 8,
                                      vertical: 8,
                                    ),
                                    decoration: BoxDecoration(
                                      color: isPlaying
                                          ? AppTheme.primaryGreen.withValues(
                                              alpha: 0.08,
                                            )
                                          : Colors.transparent,
                                      borderRadius: BorderRadius.circular(10),
                                    ),
                                    child: Row(
                                      children: [
                                        Stack(
                                          children: [
                                            ClipRRect(
                                              borderRadius:
                                                  BorderRadius.circular(6),
                                              child: item.artUri != null
                                                  ? Image.network(
                                                      item.artUri.toString(),
                                                      width: 40,
                                                      height: 40,
                                                      fit: BoxFit.cover,
                                                      errorBuilder:
                                                          (
                                                            context,
                                                            error,
                                                            stackTrace,
                                                          ) =>
                                                              _QueueArtFallback(
                                                                isPlaying:
                                                                    isPlaying,
                                                                itemId: item.id,
                                                              ),
                                                    )
                                                  : _QueueArtFallback(
                                                      isPlaying: isPlaying,
                                                      itemId: item.id,
                                                    ),
                                            ),
                                            if (isPlaying)
                                              Container(
                                                width: 40,
                                                height: 40,
                                                decoration: BoxDecoration(
                                                  color: Colors.black54,
                                                  borderRadius:
                                                      BorderRadius.circular(6),
                                                ),
                                                child: const Center(
                                                  child: SizedBox(
                                                    width: 14,
                                                    height: 14,
                                                    child: AnimatedEqualizer(
                                                      size: 'sm',
                                                    ),
                                                  ),
                                                ),
                                              ),
                                          ],
                                        ),
                                        const SizedBox(width: 10),
                                        Expanded(
                                          child: Column(
                                            crossAxisAlignment:
                                                CrossAxisAlignment.start,
                                            children: [
                                              Text(
                                                item.title,
                                                maxLines: 1,
                                                overflow: TextOverflow.ellipsis,
                                                style: TextStyle(
                                                  color: isPlaying
                                                      ? AppTheme.primaryGreen
                                                      : Colors.white,
                                                  fontWeight: isPlaying
                                                      ? FontWeight.bold
                                                      : FontWeight.normal,
                                                  fontSize: 14,
                                                ),
                                              ),
                                              const SizedBox(height: 2),
                                              Row(
                                                children: [
                                                  Flexible(
                                                    child: Text(
                                                      item.artist ??
                                                          'FreshLoop',
                                                      maxLines: 1,
                                                      overflow:
                                                          TextOverflow.ellipsis,
                                                      style: TextStyle(
                                                        color: isPlaying
                                                            ? AppTheme
                                                                  .primaryGreen
                                                                  .withValues(
                                                                    alpha: 0.7,
                                                                  )
                                                            : Colors.white38,
                                                        fontSize: 12,
                                                      ),
                                                    ),
                                                  ),
                                                  if (duration != null) ...[
                                                    const Text(
                                                      ' · ',
                                                      style: TextStyle(
                                                        color: Colors.white24,
                                                        fontSize: 12,
                                                      ),
                                                    ),
                                                    Text(
                                                      '${duration.inMinutes}:${(duration.inSeconds % 60).toString().padLeft(2, '0')}',
                                                      style: TextStyle(
                                                        color: isPlaying
                                                            ? AppTheme
                                                                  .primaryGreen
                                                                  .withValues(
                                                                    alpha: 0.7,
                                                                  )
                                                            : Colors.white38,
                                                        fontSize: 12,
                                                        fontFamily: 'monospace',
                                                      ),
                                                    ),
                                                  ],
                                                ],
                                              ),
                                            ],
                                          ),
                                        ),
                                      ],
                                    ),
                                  ),
                                ),
                              );

                              if (!isRadioItem) {
                                return queueTile;
                              }

                              return Dismissible(
                                key: ValueKey(item.id),
                                direction: DismissDirection.endToStart,
                                onDismissed: (_) {
                                  unawaited(
                                    context.read<FeedProvider>().markAsPlayed(
                                      item.id,
                                    ),
                                  );
                                },
                                background: Container(
                                  alignment: Alignment.centerRight,
                                  padding: const EdgeInsets.only(right: 20),
                                  margin: const EdgeInsets.symmetric(
                                    horizontal: 8,
                                    vertical: 8,
                                  ),
                                  decoration: BoxDecoration(
                                    color: Colors.red.withValues(alpha: 0.8),
                                    borderRadius: BorderRadius.circular(10),
                                  ),
                                  child: const Icon(
                                    Icons.delete_outline,
                                    color: Colors.white,
                                  ),
                                ),
                                child: queueTile,
                              );
                            },
                          );
                        },
                      ),
              ),
            ),
            const SizedBox(height: 16),
          ],
        ),
      ),
    );
  }
}

class _SourceLinkCard extends StatelessWidget {
  final String title;
  final String summary;
  final String buttonLabel;
  final VoidCallback onTap;

  const _SourceLinkCard({
    required this.title,
    required this.summary,
    required this.buttonLabel,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: Colors.white.withValues(alpha: 0.04),
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: Colors.white.withValues(alpha: 0.08)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            title,
            style: const TextStyle(
              color: Colors.white,
              fontSize: 14,
              fontWeight: FontWeight.w800,
            ),
          ),
          const SizedBox(height: 6),
          Text(
            summary,
            maxLines: 3,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(
              color: Colors.white54,
              fontSize: 12,
              height: 1.5,
            ),
          ),
          const SizedBox(height: 10),
          TextButton.icon(
            onPressed: onTap,
            style: TextButton.styleFrom(
              padding: EdgeInsets.zero,
              foregroundColor: AppTheme.primaryGreen,
            ),
            icon: const Icon(Icons.open_in_new, size: 16),
            label: Text(
              buttonLabel,
              style: const TextStyle(fontWeight: FontWeight.w800),
            ),
          ),
        ],
      ),
    );
  }
}

class _QueueArtFallback extends StatelessWidget {
  final bool isPlaying;
  final String itemId;

  const _QueueArtFallback({required this.isPlaying, required this.itemId});

  @override
  Widget build(BuildContext context) {
    final icon = itemId.startsWith('weekly:')
        ? Icons.newspaper
        : itemId.startsWith('curated:')
        ? Icons.headphones
        : Icons.music_note;

    return Container(
      width: 40,
      height: 40,
      decoration: BoxDecoration(
        color: isPlaying ? AppTheme.primaryGreen : AppTheme.surfaceHighlight,
        borderRadius: BorderRadius.circular(6),
      ),
      child: Icon(
        icon,
        color: isPlaying ? Colors.black : Colors.white70,
        size: 18,
      ),
    );
  }
}
