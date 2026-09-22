import 'package:flutter/material.dart';
import 'theme.dart';

class HeroCard extends StatelessWidget {
  final int unreadCount;
  final VoidCallback onRefresh;
  final bool isLoading;

  const HeroCard({
    super.key,
    required this.unreadCount,
    required this.onRefresh,
    required this.isLoading,
  });

  String _getGreeting() {
    final hour = DateTime.now().hour;
    if (hour < 5) return "夜深了";
    if (hour < 12) return "早上好";
    if (hour < 17) return "下午好";
    if (hour < 21) return "晚上好";
    return "夜间好";
  }

  @override
  Widget build(BuildContext context) {
    final now = DateTime.now();
    const weekdays = ['星期一', '星期二', '星期三', '星期四', '星期五', '星期六', '星期日'];
    final today = '${now.month}月${now.day}日 ${weekdays[now.weekday - 1]}';

    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      padding: const EdgeInsets.all(24),
      decoration: BoxDecoration(
        color: AppTheme.surfaceDark,
        borderRadius: BorderRadius.circular(24),
        border: Border.all(color: Colors.white.withValues(alpha: 0.05)),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: 0.2),
            blurRadius: 10,
            offset: const Offset(0, 4),
          ),
        ],
        gradient: RadialGradient(
          center: const Alignment(0.8, -0.8),
          radius: 1.5,
          colors: [
            AppTheme.primaryGreen.withValues(alpha: 0.15),
            Colors.transparent,
          ],
          stops: const [0.0, 0.6],
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            today,
            style: const TextStyle(
              color: AppTheme.textMuted,
              fontSize: 12,
              fontWeight: FontWeight.bold,
              letterSpacing: 1.2,
            ),
          ),
          const SizedBox(height: 4),
          Text(
            _getGreeting(),
            style: const TextStyle(
              color: Colors.white,
              fontSize: 28,
              fontWeight: FontWeight.bold,
              letterSpacing: -0.5,
            ),
          ),
          const SizedBox(height: 24),
          Row(
            children: [
              Text(
                unreadCount.toString(),
                style: const TextStyle(
                  color: AppTheme.primaryGreen,
                  fontSize: 48,
                  fontWeight: FontWeight.bold,
                  height: 1,
                ),
              ),
              const SizedBox(width: 16),
              const Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      "待听内容",
                      style: TextStyle(
                        color: Colors.white,
                        fontSize: 16,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                    Text(
                      "已为你整理",
                      style: TextStyle(color: Colors.white54, fontSize: 13),
                    ),
                  ],
                ),
              ),
              IconButton(
                onPressed: isLoading ? null : onRefresh,
                icon: isLoading
                    ? const SizedBox(
                        width: 24,
                        height: 24,
                        child: CircularProgressIndicator(
                          strokeWidth: 2,
                          color: Colors.white54,
                        ),
                      )
                    : const Icon(Icons.refresh, color: Colors.white),
                style: IconButton.styleFrom(backgroundColor: Colors.white10),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
