import '../radio_editions.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:audio_service/audio_service.dart';
import 'package:intl/intl.dart';
import '../day_playlist.dart';
import '../app_shell.dart';
import '../main.dart'; // FeedProvider and audioHandler
import '../src/rust/models.dart';
import 'theme.dart';
import 'brand_mark.dart';
import 'animated_eq.dart';
import 'focus_screen.dart';
import 'loop_screen.dart';
import 'morphing_player.dart';
import 'login_modal.dart';
import 'reading_screen.dart';

class FeedScreen extends StatefulWidget {
  const FeedScreen({super.key});

  @override
  State<FeedScreen> createState() => _FeedScreenState();
}

class _FeedScreenState extends State<FeedScreen> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      context.read<FeedProvider>().pollUpdates();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final shell = context.watch<ShellProvider>();
    final currentTab = shell.tab;
    return Scaffold(
      body: SafeArea(
        bottom: false,
        child: Stack(
          children: [
            Column(
              children: [
                _buildHeader(
                  currentTab: currentTab,
                  onChanged: context.read<ShellProvider>().selectTab,
                ),
                Expanded(
                  child: switch (currentTab) {
                    AppTab.radio => _RadioFeedList(
                      buildFeedItem: _buildFeedItem,
                    ),
                    AppTab.reading => const ReadingScreen(),
                    AppTab.loop => const LoopScreen(),
                    AppTab.focus => const FocusScreen(),
                  },
                ),
              ],
            ),
            const MorphingPlayer(),
          ],
        ),
      ),
    );
  }

  Widget _buildHeader({
    required AppTab currentTab,
    required ValueChanged<AppTab> onChanged,
  }) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      decoration: BoxDecoration(
        color: AppTheme.darkBackground.withValues(alpha: 0.95),
        border: Border(
          bottom: BorderSide(color: Colors.white.withValues(alpha: 0.05)),
        ),
      ),
      child: Column(
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Row(
                children: [
                  const FreshLoopBrandMark(),
                  const SizedBox(width: 12),
                  const Text(
                    "FreshLoop",
                    style: TextStyle(
                      fontSize: 22,
                      fontWeight: FontWeight.w900,
                      letterSpacing: -0.7,
                    ),
                  ),
                ],
              ),
              Consumer<AuthProvider>(
                builder: (context, auth, child) {
                  if (auth.isAuthenticated) {
                    return GestureDetector(
                      onTap: () {
                        showDialog(
                          context: context,
                          builder: (context) => AlertDialog(
                            backgroundColor: const Color(0xFF18181B),
                            title: Text('你好，${auth.user!.username}'),
                            content: const Text('要退出当前账户吗？'),
                            actions: [
                              TextButton(
                                onPressed: () => Navigator.pop(context),
                                child: const Text('取消'),
                              ),
                              TextButton(
                                onPressed: () {
                                  auth.logout();
                                  Navigator.pop(context);
                                },
                                child: const Text(
                                  '退出登录',
                                  style: TextStyle(color: Colors.redAccent),
                                ),
                              ),
                            ],
                          ),
                        );
                      },
                      child: Container(
                        padding: const EdgeInsets.symmetric(
                          horizontal: 12,
                          vertical: 8,
                        ),
                        decoration: BoxDecoration(
                          color: AppTheme.primaryGreen.withValues(alpha: 0.1),
                          borderRadius: BorderRadius.circular(20),
                          border: Border.all(
                            color: AppTheme.primaryGreen.withValues(alpha: 0.3),
                          ),
                        ),
                        child: Row(
                          children: [
                            const Icon(
                              Icons.person,
                              color: AppTheme.primaryGreen,
                              size: 16,
                            ),
                            const SizedBox(width: 4),
                            Text(
                              auth.user!.username,
                              style: const TextStyle(
                                color: AppTheme.primaryGreen,
                                fontWeight: FontWeight.bold,
                                fontSize: 13,
                              ),
                            ),
                          ],
                        ),
                      ),
                    );
                  }

                  return IconButton(
                    icon: const Icon(Icons.person_outline),
                    onPressed: () {
                      showDialog(
                        context: context,
                        builder: (context) => const LoginModal(),
                      );
                    },
                    style: IconButton.styleFrom(
                      backgroundColor: Colors.white10,
                    ),
                  );
                },
              ),
            ],
          ),
          const SizedBox(height: 12),
          Container(
            padding: const EdgeInsets.all(4),
            decoration: BoxDecoration(
              color: Colors.white.withValues(alpha: 0.08),
              borderRadius: BorderRadius.circular(8),
            ),
            child: Row(
              children: [
                Expanded(
                  child: _ProductLineButton(
                    icon: Icons.radio_rounded,
                    label: 'Radio',
                    selected: currentTab == AppTab.radio,
                    onTap: () => onChanged(AppTab.radio),
                  ),
                ),
                Expanded(
                  child: _ProductLineButton(
                    icon: Icons.menu_book_rounded,
                    label: 'Reading',
                    selected: currentTab == AppTab.reading,
                    onTap: () => onChanged(AppTab.reading),
                  ),
                ),
                Expanded(
                  child: _ProductLineButton(
                    icon: Icons.repeat_rounded,
                    label: 'Loop',
                    selected: currentTab == AppTab.loop,
                    onTap: () => onChanged(AppTab.loop),
                  ),
                ),
                Expanded(
                  child: _ProductLineButton(
                    icon: Icons.adjust_rounded,
                    label: 'Focus',
                    selected: currentTab == AppTab.focus,
                    onTap: () => onChanged(AppTab.focus),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildFeedItem(
    BuildContext context,
    Item item, {
    required List<Item> playlistItems,
  }) {
    return StreamBuilder<MediaItem?>(
      stream: audioHandler.mediaItem,
      builder: (context, mediaSnapshot) {
        final currentMedia = mediaSnapshot.data;
        final isActive = currentMedia?.id == item.id;

        return StreamBuilder<PlaybackState>(
          stream: audioHandler.playbackState,
          builder: (context, stateSnapshot) {
            final playing = isActive && (stateSnapshot.data?.playing ?? false);

            // Format time and duration
            String category = item.category ?? "News";
            String title = item.title;
            if (item.category == null) {
              final match = RegExp(r'^【(.*?)】').firstMatch(title);
              if (match != null) category = match.group(1) ?? "News";
            }
            title = title.replaceAll(RegExp(r'^【.*?】'), '').trim();

            final dateObj = item.publishTime != null
                ? DateTime.fromMillisecondsSinceEpoch(item.publishTime! * 1000)
                : DateTime.now();
            final timeStr = DateFormat('MMM d, HH:mm').format(dateObj);

            final durationStr = item.durationSec != null
                ? "${(item.durationSec! / 60).floor()}:${(item.durationSec! % 60).floor().toString().padLeft(2, '0')}"
                : "Brief";
            final shell = context.read<ShellProvider>();

            return GestureDetector(
              onTap: () {
                if (!isActive) {
                  context.read<FeedProvider>().playDay(
                    playlistItems,
                    startItemId: item.id,
                  );
                } else {
                  playing ? audioHandler.pause() : audioHandler.play();
                }
              },
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 12),
                child: Row(
                  children: [
                    // Icon Box
                    Stack(
                      clipBehavior: Clip.none,
                      children: [
                        Container(
                          width: 56,
                          height: 56,
                          decoration: BoxDecoration(
                            color: isActive
                                ? AppTheme.primaryGreen
                                : const Color(0xFF244732),
                            borderRadius: BorderRadius.circular(16),
                          ),
                          child: Center(
                            child: playing
                                ? AnimatedEqualizer(
                                    size: 'lg',
                                    color: Colors.black87,
                                  )
                                : Icon(
                                    Icons.graphic_eq,
                                    color: isActive
                                        ? Colors.black87
                                        : Colors.white,
                                    size: 28,
                                  ),
                          ),
                        ),
                        Positioned(
                          top: -6,
                          left: -6,
                          child: Container(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 6,
                              vertical: 2,
                            ),
                            decoration: BoxDecoration(
                              color: Colors.black87,
                              borderRadius: BorderRadius.circular(6),
                              border: Border.all(color: Colors.white24),
                            ),
                            child: Text(
                              category.length > 4
                                  ? category.substring(0, 4)
                                  : category,
                              style: const TextStyle(
                                fontSize: 9,
                                fontWeight: FontWeight.bold,
                                color: Colors.white70,
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(width: 16),
                    // Content
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            title,
                            maxLines: 2,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontSize: 16,
                              fontWeight: FontWeight.bold,
                              color: isActive
                                  ? AppTheme.primaryGreen
                                  : Colors.white,
                              height: 1.2,
                            ),
                          ),
                          const SizedBox(height: 6),
                          Row(
                            children: [
                              const Icon(
                                Icons.schedule,
                                size: 12,
                                color: Colors.white54,
                              ),
                              const SizedBox(width: 4),
                              Text(
                                timeStr,
                                style: const TextStyle(
                                  fontSize: 11,
                                  color: Colors.white54,
                                ),
                              ),
                              const SizedBox(width: 12),
                              const Icon(
                                Icons.timer_outlined,
                                size: 12,
                                color: Colors.white54,
                              ),
                              const SizedBox(width: 4),
                              Text(
                                durationStr,
                                style: const TextStyle(
                                  fontSize: 11,
                                  color: Colors.white54,
                                ),
                              ),
                            ],
                          ),
                        ],
                      ),
                    ),
                    // Action Buttons
                    const SizedBox(width: 8),
                    Column(
                      children: [
                        Container(
                          width: 40,
                          height: 40,
                          decoration: BoxDecoration(
                            color: playing
                                ? AppTheme.primaryGreen
                                : Colors.black26,
                            shape: BoxShape.circle,
                          ),
                          child: Icon(
                            playing ? Icons.pause : Icons.play_arrow,
                            color: playing ? Colors.black : Colors.white,
                          ),
                        ),
                        const SizedBox(height: 8),
                        InkWell(
                          onTap: () {
                            shell.openLoopWithDraft(
                              LoopComposeDraft(
                                title: title,
                                references: [
                                  LoopDraftReference(
                                    sourceType: 'radio_item',
                                    sourceId: item.id,
                                    sourceUrl: item.originalUrl,
                                    title: title,
                                    quoteText: item.summary,
                                  ),
                                ],
                              ),
                            );
                          },
                          borderRadius: BorderRadius.circular(999),
                          child: Container(
                            width: 32,
                            height: 32,
                            decoration: const BoxDecoration(
                              color: Colors.black26,
                              shape: BoxShape.circle,
                            ),
                            child: const Icon(
                              Icons.format_quote,
                              color: Colors.white70,
                              size: 18,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            );
          },
        );
      },
    );
  }
}

class _ProductLineButton extends StatelessWidget {
  final IconData icon;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  const _ProductLineButton({
    required this.icon,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final foreground = selected ? Colors.black : Colors.white70;
    return TextButton(
      onPressed: onTap,
      style: TextButton.styleFrom(
        backgroundColor: selected ? AppTheme.primaryGreen : Colors.transparent,
        foregroundColor: foreground,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 8),
        minimumSize: const Size(0, 54),
        tapTargetSize: MaterialTapTargetSize.shrinkWrap,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: 16, color: foreground),
          const SizedBox(height: 4),
          FittedBox(
            fit: BoxFit.scaleDown,
            child: Text(
              label,
              maxLines: 1,
              softWrap: false,
              style: TextStyle(
                color: foreground,
                fontSize: 11.5,
                height: 1,
                fontWeight: FontWeight.w900,
                letterSpacing: 0,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _RadioFeedList extends StatelessWidget {
  final Widget Function(
    BuildContext context,
    Item item, {
    required List<Item> playlistItems,
  })
  buildFeedItem;

  const _RadioFeedList({required this.buildFeedItem});

  @override
  Widget build(BuildContext context) {
    return Consumer<FeedProvider>(
      builder: (context, provider, child) {
        return RefreshIndicator(
          color: AppTheme.primaryGreen,
          backgroundColor: AppTheme.surfaceHighlight,
          onRefresh: () async {
            provider.refresh();
          },
          child: ListView(
            padding: const EdgeInsets.only(bottom: 120),
            children: [
              if (provider.error != null && provider.items.isEmpty)
                _RadioStatusPanel(
                  icon: Icons.cloud_off_outlined,
                  text: '新闻暂时加载失败',
                  detail: provider.error,
                  actionLabel: '重试',
                  onAction: provider.refresh,
                )
              else if (provider.dayGroups.isEmpty &&
                  !provider.isLoading &&
                  provider.hasLoadedItems)
                const _RadioStatusPanel(
                  icon: Icons.check_circle_outline,
                  text: '暂无待处理内容',
                  detail: '新的音频生成后会自动出现在这里',
                )
              else
                ...provider.dayGroups.map(
                  (group) => _RadioDaySection(
                    key: ValueKey(group.key),
                    group: group,
                    buildFeedItem: buildFeedItem,
                  ),
                ),
              if (provider.isLoading)
                const Center(
                  child: Padding(
                    padding: EdgeInsets.all(16.0),
                    child: CircularProgressIndicator(
                      color: AppTheme.primaryGreen,
                    ),
                  ),
                )
              else
                Padding(
                  padding: const EdgeInsets.all(16.0),
                  child: TextButton(
                    onPressed: provider.fetchItems,
                    style: TextButton.styleFrom(
                      foregroundColor: AppTheme.primaryGreen,
                    ),
                    child: const Text('Load More'),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}

class _RadioStatusPanel extends StatelessWidget {
  final IconData icon;
  final String text;
  final String? detail;
  final String? actionLabel;
  final VoidCallback? onAction;

  const _RadioStatusPanel({
    required this.icon,
    required this.text,
    this.detail,
    this.actionLabel,
    this.onAction,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(24),
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.all(18),
        decoration: BoxDecoration(
          color: Colors.white.withValues(alpha: 0.04),
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: Colors.white.withValues(alpha: 0.08)),
        ),
        child: Column(
          children: [
            Icon(icon, color: AppTheme.primaryGreen, size: 30),
            const SizedBox(height: 10),
            Text(
              text,
              style: const TextStyle(
                color: Colors.white,
                fontWeight: FontWeight.w800,
              ),
              textAlign: TextAlign.center,
            ),
            if (detail != null && detail!.isNotEmpty) ...[
              const SizedBox(height: 8),
              Text(
                detail!,
                style: const TextStyle(color: Colors.white54, height: 1.35),
                textAlign: TextAlign.center,
              ),
            ],
            if (actionLabel != null && onAction != null) ...[
              const SizedBox(height: 12),
              TextButton(
                onPressed: onAction,
                style: TextButton.styleFrom(
                  foregroundColor: AppTheme.primaryGreen,
                ),
                child: Text(actionLabel!),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _RadioDaySection extends StatefulWidget {
  final DayPlaylistGroup<Item> group;
  final Widget Function(
    BuildContext context,
    Item item, {
    required List<Item> playlistItems,
  })
  buildFeedItem;
  const _RadioDaySection({
    required this.group,
    required this.buildFeedItem,
    super.key,
  });
  @override
  State<_RadioDaySection> createState() => _RadioDaySectionState();
}

class _RadioDaySectionState extends State<_RadioDaySection> {
  bool expanded = false;
  @override
  Widget build(BuildContext context) {
    final group = widget.group;
    final provider = context.watch<FeedProvider>();
    final added = group.playbackIds.where(provider.newAudioIds.contains).length;
    final duration =
        '${group.totalDurationSec ~/ 60}:${(group.totalDurationSec % 60).toString().padLeft(2, '0')}';
    return Container(
      margin: const EdgeInsets.fromLTRB(16, 12, 16, 0),
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: AppTheme.surfaceDark,
        borderRadius: BorderRadius.circular(20),
        border: Border.all(color: Colors.white.withValues(alpha: 0.06)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      group.shortTitle,
                      style: const TextStyle(
                        fontSize: 16,
                        fontWeight: FontWeight.w900,
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      group.title.split(' · ').last,
                      style: const TextStyle(
                        fontSize: 13,
                        color: Colors.white70,
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      '${group.items.length == 1 && isRadioProgram(group.items.first) ? '${programSectionCount(group.items.first)} 个板块 · 完整节目' : '${group.playableCount} 个音频'} · $duration${group.items.length > group.playableCount ? ' · ${group.items.length - group.playableCount} 篇待生成' : ''}',
                      style: const TextStyle(
                        fontSize: 12,
                        color: Colors.white54,
                      ),
                    ),
                    if (added > 0)
                      Semantics(
                        liveRegion: true,
                        child: Text(
                          '新增 $added 个音频',
                          style: const TextStyle(
                            color: AppTheme.primaryGreen,
                            fontSize: 12,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              IconButton.filled(
                tooltip: '播放节目',
                style: IconButton.styleFrom(
                  backgroundColor: AppTheme.primaryGreen,
                  foregroundColor: Colors.black,
                ),
                onPressed: group.playableCount == 0
                    ? null
                    : () => provider.playDay(group.items),
                icon: const Icon(Icons.play_arrow_rounded),
              ),
            ],
          ),
          TextButton(
            onPressed: () => setState(() => expanded = !expanded),
            child: Text(expanded ? '收起节目' : '查看节目'),
          ),
          if (expanded)
            ...group.items.map(
              (item) => widget.buildFeedItem(
                context,
                item,
                playlistItems: group.items,
              ),
            ),
        ],
      ),
    );
  }
}
