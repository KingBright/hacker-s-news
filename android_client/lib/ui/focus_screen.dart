import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../loop_api.dart';
import '../main.dart';
import 'login_modal.dart';
import 'theme.dart';

class FocusScreen extends StatefulWidget {
  const FocusScreen({super.key});

  @override
  State<FocusScreen> createState() => _FocusScreenState();
}

class _FocusScreenState extends State<FocusScreen> {
  final LoopApi _api = LoopApi(baseUrl: baseUrl);
  FocusSummary? _summary;
  bool _loading = true;
  String? _error;
  String? _loadedUserId;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final user = context.read<AuthProvider>().user;
    if (user != null && user.id != _loadedUserId) {
      _loadedUserId = user.id;
      _load(user.id);
    }
    if (user == null) {
      _loadedUserId = null;
      _summary = null;
      if (_loading) {
        setState(() => _loading = false);
      }
    }
  }

  Future<void> _load(String userId) async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final summary = await _api.fetchFocus(userId);
      if (!mounted) return;
      setState(() => _summary = summary);
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final auth = context.watch<AuthProvider>();
    return RefreshIndicator(
      color: AppTheme.primaryGreen,
      backgroundColor: AppTheme.surfaceDark,
      onRefresh: () async {
        if (auth.user != null) {
          await _load(auth.user!.id);
        }
      },
      child: ListView(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 120),
        children: [
          Container(
            padding: const EdgeInsets.all(18),
            decoration: BoxDecoration(
              color: AppTheme.surfaceDark,
              borderRadius: BorderRadius.circular(20),
              border: Border.all(color: Colors.white.withValues(alpha: 0.06)),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'Focus',
                  style: TextStyle(fontSize: 24, fontWeight: FontWeight.w900),
                ),
                const SizedBox(height: 8),
                const Text(
                  '这里展示当前注意力配比：近期表达提高侧重，长期兴趣保持稳定，探索位始终保留。',
                  style: TextStyle(color: Colors.white60, height: 1.5),
                ),
                if (_summary != null) ...[
                  const SizedBox(height: 16),
                  _BalanceCard(
                    title: 'Reading Mix',
                    balance: _summary!.readingBalance,
                  ),
                  const SizedBox(height: 10),
                  _BalanceCard(
                    title: 'Radio Mix',
                    balance: _summary!.radioBalance,
                  ),
                  const SizedBox(height: 10),
                  _StatusCard(
                    title: 'Loop Status',
                    note:
                        '表达 ${_summary!.stats.expressionCount} · 已吸收 ${_summary!.stats.processedExpressionCount} · 待提炼 ${_summary!.stats.pendingExpressionCount} · 信号 ${_summary!.stats.signalCount}',
                  ),
                ],
              ],
            ),
          ),
          const SizedBox(height: 18),
          if (!auth.isAuthenticated)
            Container(
              padding: const EdgeInsets.all(18),
              decoration: BoxDecoration(
                color: Colors.white.withValues(alpha: 0.04),
                borderRadius: BorderRadius.circular(16),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text(
                    '登录后才能看到 Focus',
                    style: TextStyle(fontWeight: FontWeight.w900),
                  ),
                  const SizedBox(height: 8),
                  const Text(
                    '这样系统才能把你的表达、偏好提炼和来源倾向汇总成可见画像。',
                    style: TextStyle(color: Colors.white60, height: 1.5),
                  ),
                  const SizedBox(height: 12),
                  OutlinedButton(
                    onPressed: () {
                      showDialog(
                        context: context,
                        builder: (_) => const LoginModal(),
                      );
                    },
                    child: const Text('去登录'),
                  ),
                ],
              ),
            )
          else if (_loading)
            const Center(
              child: Padding(
                padding: EdgeInsets.all(24),
                child: CircularProgressIndicator(color: AppTheme.primaryGreen),
              ),
            )
          else if (_error != null)
            Text(_error!, style: const TextStyle(color: Colors.redAccent))
          else if (_summary != null) ...[
            _FocusGroup(
              title: '当前焦点',
              detail: '更影响接下来几轮的排序和摘要侧重点。',
              items: _summary!.currentFocus,
            ),
            _FocusGroup(
              title: '长期兴趣',
              detail: '反复出现、相对稳定的方向。',
              items: _summary!.longTermFocus,
            ),
            _FocusGroup(
              title: '最近降低',
              detail: '只是阶段性调低，不代表彻底不看。',
              items: _summary!.recentlyReduced,
            ),
            _FocusGroup(
              title: '偏好来源',
              detail: '你最近更容易被这些来源触发表达。',
              items: _summary!.preferredSources,
            ),
            _FocusGroup(
              title: '偏好形态',
              detail: '系统判断近期更适合用什么媒介承载。',
              items: _summary!.preferredFormats,
            ),
          ],
        ],
      ),
    );
  }
}

class _FocusGroup extends StatelessWidget {
  final String title;
  final String detail;
  final List<FocusCard> items;

  const _FocusGroup({
    required this.title,
    required this.detail,
    required this.items,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.only(bottom: 12),
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: AppTheme.surfaceDark,
        borderRadius: BorderRadius.circular(20),
        border: Border.all(color: Colors.white.withValues(alpha: 0.06)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            title,
            style: const TextStyle(fontSize: 20, fontWeight: FontWeight.w900),
          ),
          const SizedBox(height: 6),
          Text(
            detail,
            style: const TextStyle(color: Colors.white60, height: 1.5),
          ),
          const SizedBox(height: 12),
          if (items.isEmpty)
            const Text('还没有足够的信号', style: TextStyle(color: Colors.white54))
          else
            ...items.map(
              (item) => Container(
                margin: const EdgeInsets.only(bottom: 10),
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: Colors.black26,
                  borderRadius: BorderRadius.circular(14),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            item.label,
                            style: const TextStyle(fontWeight: FontWeight.w900),
                          ),
                        ),
                        Text(
                          item.score.toStringAsFixed(2),
                          style: const TextStyle(color: AppTheme.primaryGreen),
                        ),
                      ],
                    ),
                    const SizedBox(height: 6),
                    Text(
                      focusKindLabel(item.kind),
                      style: const TextStyle(
                        fontSize: 11,
                        color: Colors.white38,
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      item.evidence,
                      style: const TextStyle(
                        color: Colors.white60,
                        height: 1.5,
                      ),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _BalanceCard extends StatelessWidget {
  final String title;
  final BalanceRule balance;

  const _BalanceCard({required this.title, required this.balance});

  @override
  Widget build(BuildContext context) {
    final segments = [
      (label: '近期', value: balance.activePct, color: AppTheme.primaryGreen),
      (label: '长期', value: balance.stablePct, color: AppTheme.textMuted),
      (label: '探索', value: balance.explorePct, color: Colors.white54),
    ];

    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: Colors.black26,
        borderRadius: BorderRadius.circular(14),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            title,
            style: const TextStyle(
              fontWeight: FontWeight.w900,
              color: AppTheme.primaryGreen,
            ),
          ),
          const SizedBox(height: 10),
          ClipRRect(
            borderRadius: BorderRadius.circular(999),
            child: Row(
              children: segments
                  .map(
                    (segment) => Expanded(
                      flex: segment.value <= 0 ? 1 : segment.value,
                      child: Container(height: 7, color: segment.color),
                    ),
                  )
                  .toList(growable: false),
            ),
          ),
          const SizedBox(height: 8),
          Wrap(
            spacing: 10,
            runSpacing: 4,
            children: segments
                .map(
                  (segment) => Text(
                    '${segment.value}% ${segment.label}',
                    style: const TextStyle(color: Colors.white54, fontSize: 11),
                  ),
                )
                .toList(growable: false),
          ),
          const SizedBox(height: 8),
          Text(
            balance.note,
            style: const TextStyle(color: Colors.white70, height: 1.5),
          ),
        ],
      ),
    );
  }
}

class _StatusCard extends StatelessWidget {
  final String title;
  final String note;

  const _StatusCard({required this.title, required this.note});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: Colors.black26,
        borderRadius: BorderRadius.circular(14),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            title,
            style: const TextStyle(
              fontWeight: FontWeight.w900,
              color: AppTheme.primaryGreen,
            ),
          ),
          const SizedBox(height: 6),
          Text(
            note,
            style: const TextStyle(color: Colors.white70, height: 1.5),
          ),
        ],
      ),
    );
  }
}
