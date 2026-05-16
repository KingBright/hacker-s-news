import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:audio_service/audio_service.dart';
import 'package:intl/intl.dart';
import '../main.dart'; // FeedProvider and audioHandler
import '../src/rust/models.dart';
import 'theme.dart';
import 'hero_card.dart';
import 'animated_eq.dart';
import 'morphing_player.dart';
import 'login_modal.dart';

class FeedScreen extends StatelessWidget {
  const FeedScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        bottom: false,
        child: Stack(
          children: [
            Column(
              children: [
                _buildHeader(),
                Expanded(
                  child: Consumer<FeedProvider>(
                    builder: (context, provider, child) {
                      return RefreshIndicator(
                        color: AppTheme.primaryGreen,
                        backgroundColor: AppTheme.surfaceHighlight,
                        onRefresh: () async {
                          provider.page = 1;
                          provider.items.clear();
                          await provider.fetchItems();
                        },
                        child: ListView.builder(
                          padding: const EdgeInsets.only(
                            bottom: 120,
                          ), // Space for morphing player
                          itemCount:
                              provider.items.length +
                              2, // HeroCard + Items + LoadMore
                          itemBuilder: (context, index) {
                            if (index == 0) {
                              return HeroCard(
                                unreadCount: provider.items.length,
                                onRefresh: () {
                                  provider.page = 1;
                                  provider.items.clear();
                                  provider.fetchItems();
                                },
                                isLoading:
                                    provider.isLoading &&
                                    provider.items.isEmpty,
                              );
                            }

                            final itemIndex = index - 1;
                            if (itemIndex == provider.items.length) {
                              if (provider.isLoading) {
                                return const Center(
                                  child: Padding(
                                    padding: EdgeInsets.all(16.0),
                                    child: CircularProgressIndicator(
                                      color: AppTheme.primaryGreen,
                                    ),
                                  ),
                                );
                              }
                              return Padding(
                                padding: const EdgeInsets.all(16.0),
                                child: TextButton(
                                  onPressed: provider.fetchItems,
                                  style: TextButton.styleFrom(
                                    foregroundColor: AppTheme.primaryGreen,
                                  ),
                                  child: const Text('Load More'),
                                ),
                              );
                            }

                            final item = provider.items[itemIndex];
                            return _buildFeedItem(context, item, itemIndex);
                          },
                        ),
                      );
                    },
                  ),
                ),
              ],
            ),
            const MorphingPlayer(),
          ],
        ),
      ),
    );
  }

  Widget _buildHeader() {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      decoration: BoxDecoration(
        color: AppTheme.darkBackground.withValues(alpha: 0.95),
        border: Border(
          bottom: BorderSide(color: Colors.white.withValues(alpha: 0.05)),
        ),
      ),
      child: Row(
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
                child: const Icon(Icons.waves, color: AppTheme.primaryGreen),
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
                    "AUDIO BRIEFING • ZEN MODE",
                    style: TextStyle(
                      fontSize: 9,
                      color: AppTheme.textMuted,
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
                style: IconButton.styleFrom(backgroundColor: Colors.white10),
              );
            },
          ),
        ],
      ),
    );
  }

  Widget _buildFeedItem(BuildContext context, Item item, int index) {
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

            return GestureDetector(
              onTap: () {
                if (!isActive) {
                  audioHandler.skipToQueueItem(index);
                  audioHandler.play();
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
                    Container(
                      width: 40,
                      height: 40,
                      decoration: BoxDecoration(
                        color: playing ? AppTheme.primaryGreen : Colors.black26,
                        shape: BoxShape.circle,
                      ),
                      child: Icon(
                        playing ? Icons.pause : Icons.play_arrow,
                        color: playing ? Colors.black : Colors.white,
                      ),
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
