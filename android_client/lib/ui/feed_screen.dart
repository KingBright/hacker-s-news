import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:audio_service/audio_service.dart';
import 'package:intl/intl.dart';
import '../day_playlist.dart';
import '../app_shell.dart';
import '../main.dart'; // FeedProvider and audioHandler
import '../src/rust/models.dart';
import 'theme.dart';
import 'hero_card.dart';
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

class _FeedScreenState extends State<FeedScreen> {
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
                  Container(
                    width: 40,
                    height: 40,
                    decoration: BoxDecoration(
                      borderRadius: BorderRadius.circular(12),
                      color: AppTheme.surfaceHighlight,
                    ),
                    child: const Icon(
                      Icons.waves,
                      color: AppTheme.primaryGreen,
                    ),
                  ),
                  const SizedBox(width: 12),
                  const Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        "FreshLoop",
                        style: TextStyle(
                          fontSize: 22,
                          fontWeight: FontWeight.bold,
                          letterSpacing: -0.5,
                        ),
                      ),
                      Text(
                        "RADIO + READING",
                        style: TextStyle(
                          fontSize: 9,
                          color: AppTheme.textMuted,
                          fontWeight: FontWeight.bold,
                          letterSpacing: 1.0,
                        ),
                      ),
                      Text(
                        "LOOP + FOCUS",
                        style: TextStyle(
                          fontSize: 9,
                          color: Colors.white38,
                          fontWeight: FontWeight.bold,
                          letterSpacing: 1.0,
                        ),
                      ),
                    ],
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
                            title: Text('Hi, ${auth.user!.username}'),
                            content: const Text('Do you want to log out?'),
                            actions: [
                              TextButton(
                                onPressed: () => Navigator.pop(context),
                                child: const Text('Cancel'),
                              ),
                              TextButton(
                                onPressed: () {
                                  auth.logout();
                                  Navigator.pop(context);
                                },
                                child: const Text(
                                  'Log Out',
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
                    icon: Icons.radio,
                    label: 'Radio',
                    selected: currentTab == AppTab.radio,
                    onTap: () => onChanged(AppTab.radio),
                  ),
                ),
                Expanded(
                  child: _ProductLineButton(
                    icon: Icons.menu_book,
                    label: 'Reading',
                    selected: currentTab == AppTab.reading,
                    onTap: () => onChanged(AppTab.reading),
                  ),
                ),
                Expanded(
                  child: _ProductLineButton(
                    icon: Icons.format_quote,
                    label: 'Loop',
                    selected: currentTab == AppTab.loop,
                    onTap: () => onChanged(AppTab.loop),
                  ),
                ),
                Expanded(
                  child: _ProductLineButton(
                    icon: Icons.adjust,
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
              child: Container(
                margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
                padding: const EdgeInsets.all(16),
                decoration: BoxDecoration(
                  color: AppTheme.surfaceDark,
                  borderRadius: BorderRadius.circular(20),
                  border: Border.all(
                    color: isActive
                        ? AppTheme.primaryGreen
                        : Colors.white.withValues(alpha: 0.05),
                    width: isActive ? 1.5 : 1.0,
                  ),
                ),
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
              HeroCard(
                unreadCount: provider.items.length,
                onRefresh: provider.refresh,
                isLoading: provider.isLoading && provider.items.isEmpty,
              ),
              if (provider.items.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 10, 16, 0),
                  child: Align(
                    alignment: Alignment.centerRight,
                    child: TextButton.icon(
                      onPressed: () => provider.playWholeQueue(),
                      style: TextButton.styleFrom(
                        foregroundColor: AppTheme.primaryGreen,
                      ),
                      icon: const Icon(Icons.play_circle_fill_rounded),
                      label: const Text('全部播放'),
                    ),
                  ),
                ),
              if (provider.dayGroups.isEmpty && !provider.isLoading)
                const Padding(
                  padding: EdgeInsets.all(24),
                  child: Center(
                    child: Text(
                      '暂无待处理内容',
                      style: TextStyle(color: Colors.white54),
                    ),
                  ),
                )
              else
                ...provider.dayGroups.map(
                  (group) => _RadioDaySection(
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

class _RadioDaySection extends StatelessWidget {
  final DayPlaylistGroup<Item> group;
  final Widget Function(
    BuildContext context,
    Item item, {
    required List<Item> playlistItems,
  })
  buildFeedItem;

  const _RadioDaySection({required this.group, required this.buildFeedItem});

  @override
  Widget build(BuildContext context) {
    final playlistItems = [...group.items]
      ..sort((left, right) {
        final leftTime = left.publishTime ?? left.createdAt ?? 0;
        final rightTime = right.publishTime ?? right.createdAt ?? 0;
        final byTime = leftTime.compareTo(rightTime);
        if (byTime != 0) return byTime;
        return left.id.compareTo(right.id);
      });
    final durationMinutes = group.totalDurationSec > 0
        ? (group.totalDurationSec / 60).ceil()
        : 0;

    return Container(
      margin: const EdgeInsets.fromLTRB(16, 12, 16, 0),
      padding: const EdgeInsets.fromLTRB(14, 14, 14, 8),
      decoration: BoxDecoration(
        color: Colors.white.withValues(alpha: 0.03),
        borderRadius: BorderRadius.circular(24),
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
                      '${group.items.length} 条内容 · ${group.playableCount} 段可播${durationMinutes > 0 ? ' · $durationMinutes min' : ''}',
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
                  onPressed: () {
                    context.read<FeedProvider>().playDay(playlistItems);
                  },
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
          const SizedBox(height: 8),
          ...playlistItems.map(
            (item) =>
                buildFeedItem(context, item, playlistItems: playlistItems),
          ),
        ],
      ),
    );
  }
}
