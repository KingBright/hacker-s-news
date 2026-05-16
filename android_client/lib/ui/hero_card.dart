import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
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
    if (hour < 5) return "Good Late Night";
    if (hour < 12) return "Good Morning";
    if (hour < 17) return "Good Afternoon";
    if (hour < 21) return "Good Evening";
    return "Good Night";
  }

  @override
  Widget build(BuildContext context) {
    final today = DateFormat('EEEE, MMM d').format(DateTime.now());

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
            today.toUpperCase(),
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
                      "Fresh stories",
                      style: TextStyle(
                        color: Colors.white,
                        fontSize: 16,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                    Text(
                      "Tailored for you",
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
