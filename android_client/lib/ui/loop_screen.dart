import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:url_launcher/url_launcher.dart';

import '../app_shell.dart';
import '../loop_api.dart';
import '../main.dart';
import 'login_modal.dart';
import 'theme.dart';

class LoopScreen extends StatefulWidget {
  const LoopScreen({super.key});

  @override
  State<LoopScreen> createState() => _LoopScreenState();
}

class _LoopScreenState extends State<LoopScreen> {
  final LoopApi _api = LoopApi(baseUrl: baseUrl);
  final TextEditingController _titleController = TextEditingController();
  final TextEditingController _bodyController = TextEditingController();
  List<LoopPostEntry> _posts = const [];
  bool _loading = true;
  bool _submitting = false;
  String? _error;
  String _feedbackMode = 'balance';
  String? _draftSignature;
  String? _loadedUserId;

  @override
  void dispose() {
    _titleController.dispose();
    _bodyController.dispose();
    super.dispose();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final auth = context.read<AuthProvider>();
    final shell = context.watch<ShellProvider>();
    final draft = shell.loopDraft;
    final signature = draft == null
        ? null
        : '${draft.feedbackMode}:${draft.title}:${draft.references.length}';
    if (_draftSignature != signature) {
      _draftSignature = signature;
      _titleController.text = draft?.title ?? '';
      _bodyController.text = draft?.body ?? '';
      _feedbackMode = draft?.feedbackMode ?? 'balance';
    }
    if (auth.user != null && auth.user!.id != _loadedUserId) {
      _loadedUserId = auth.user!.id;
      _loadPosts(auth.user!.id);
    }
    if (auth.user == null) {
      _loadedUserId = null;
      _posts = const [];
      if (_loading) {
        setState(() => _loading = false);
      }
    }
  }

  Future<void> _loadPosts(String userId) async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final posts = await _api.fetchPosts(userId);
      if (!mounted) return;
      setState(() => _posts = posts);
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  Future<void> _publish() async {
    final auth = context.read<AuthProvider>();
    final shell = context.read<ShellProvider>();
    final draft = shell.loopDraft;
    final user = auth.user;
    if (user == null) {
      showDialog(context: context, builder: (_) => const LoginModal());
      return;
    }
    final body = _bodyController.text.trim();
    if (body.isEmpty) {
      setState(() => _error = '请先写下你的表达');
      return;
    }

    setState(() {
      _submitting = true;
      _error = null;
    });
    try {
      final created = await _api.createPost(
        userId: user.id,
        title: _titleController.text.trim().isEmpty
            ? null
            : _titleController.text.trim(),
        body: body,
        feedbackMode: _feedbackMode,
        sourceRef: _buildSourceRef(draft?.references ?? const []),
        references: (draft?.references ?? const []).map((reference) {
          return {
            'source_type': reference.sourceType,
            'source_id': reference.sourceId,
            'source_url': reference.sourceUrl,
            'title': reference.title,
            'quote_text': reference.quoteText,
            'start_ms': reference.startMs,
            'end_ms': reference.endMs,
          };
        }).toList(),
      );
      if (!mounted) return;
      setState(() {
        _posts = [created, ..._posts];
        _titleController.clear();
        _bodyController.clear();
        _feedbackMode = 'balance';
      });
      shell.clearLoopDraft();
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _submitting = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final auth = context.watch<AuthProvider>();
    final draft = context.watch<ShellProvider>().loopDraft;

    return RefreshIndicator(
      color: AppTheme.primaryGreen,
      backgroundColor: AppTheme.surfaceDark,
      onRefresh: () async {
        if (auth.user != null) {
          await _loadPosts(auth.user!.id);
        }
      },
      child: ListView(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 120),
        children: [
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              color: AppTheme.surfaceDark,
              borderRadius: BorderRadius.circular(20),
              border: Border.all(color: Colors.white.withValues(alpha: 0.06)),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'Loop',
                  style: TextStyle(fontSize: 24, fontWeight: FontWeight.w900),
                ),
                const SizedBox(height: 6),
                const Text(
                  '像引用转发一样写下判断。FreshLoop 会调节侧重点，但不会把内容砍成黑白两类。',
                  style: TextStyle(color: Colors.white60, height: 1.5),
                ),
                const SizedBox(height: 14),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    _ModeChip(
                      label: '平衡记录',
                      selected: _feedbackMode == 'balance',
                      onTap: () => setState(() => _feedbackMode = 'balance'),
                    ),
                    _ModeChip(
                      label: '加重一点',
                      selected: _feedbackMode == 'boost',
                      onTap: () => setState(() => _feedbackMode = 'boost'),
                    ),
                    _ModeChip(
                      label: '降一点',
                      selected: _feedbackMode == 'reduce',
                      onTap: () => setState(() => _feedbackMode = 'reduce'),
                    ),
                    _ModeChip(
                      label: '仅观察',
                      selected: _feedbackMode == 'observe',
                      onTap: () => setState(() => _feedbackMode = 'observe'),
                    ),
                  ],
                ),
                if (draft != null && draft.references.isNotEmpty) ...[
                  const SizedBox(height: 16),
                  const Text(
                    '引用原内容',
                    style: TextStyle(
                      color: AppTheme.primaryGreen,
                      fontSize: 13,
                      fontWeight: FontWeight.w900,
                    ),
                  ),
                  const SizedBox(height: 10),
                  ...draft.references.map(
                    (reference) => Padding(
                      padding: const EdgeInsets.only(bottom: 10),
                      child: _ReferenceCard(reference: reference),
                    ),
                  ),
                ],
                const SizedBox(height: 10),
                TextField(
                  controller: _titleController,
                  decoration: const InputDecoration(
                    hintText: '标题可选',
                    filled: true,
                    fillColor: Colors.black26,
                    border: OutlineInputBorder(borderSide: BorderSide.none),
                  ),
                ),
                const SizedBox(height: 10),
                TextField(
                  controller: _bodyController,
                  maxLines: 7,
                  decoration: const InputDecoration(
                    hintText: '直接说你的判断、偏好或边界',
                    filled: true,
                    fillColor: Colors.black26,
                    border: OutlineInputBorder(borderSide: BorderSide.none),
                  ),
                ),
                if (_error != null) ...[
                  const SizedBox(height: 10),
                  Text(
                    _error!,
                    style: const TextStyle(color: Colors.redAccent),
                  ),
                ],
                const SizedBox(height: 12),
                Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    const Expanded(
                      child: Text(
                        '你的表达会参与下一轮排序，长期信号整理后会出现在 Focus。',
                        style: TextStyle(color: Colors.white54, fontSize: 12),
                      ),
                    ),
                    const SizedBox(width: 12),
                    FilledButton(
                      onPressed: _submitting ? null : _publish,
                      style: FilledButton.styleFrom(
                        backgroundColor: AppTheme.primaryGreen,
                        foregroundColor: Colors.black,
                      ),
                      child: Text(_submitting ? '发布中...' : '发布'),
                    ),
                  ],
                ),
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
                    '登录后才能保存 Loop',
                    style: TextStyle(fontWeight: FontWeight.w900),
                  ),
                  const SizedBox(height: 8),
                  const Text(
                    '这样你的表达才能真正参与后续排序和画像更新。',
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
          else if (_posts.isEmpty)
            Container(
              padding: const EdgeInsets.all(18),
              decoration: BoxDecoration(
                color: Colors.white.withValues(alpha: 0.04),
                borderRadius: BorderRadius.circular(16),
              ),
              child: const Text(
                '还没有表达记录。你可以从 Radio 或 Reading 里引用一条内容，再回到这里补上你的判断。',
                style: TextStyle(color: Colors.white60, height: 1.6),
              ),
            )
          else
            ..._posts.map((post) => _LoopPostCard(post: post)),
        ],
      ),
    );
  }
}

String? _buildSourceRef(List<LoopDraftReference> references) {
  if (references.isEmpty) return null;
  final first = references.first;
  if (first.sourceId != null && first.sourceId!.isNotEmpty) {
    return '${first.sourceType}:${first.sourceId}';
  }
  if (first.sourceUrl != null && first.sourceUrl!.isNotEmpty) {
    return '${first.sourceType}:${first.sourceUrl}';
  }
  return null;
}

Future<void> _openLoopSourceUrl(BuildContext context, String? rawUrl) async {
  final value = rawUrl?.trim();
  if (value == null || value.isEmpty) return;
  final uri = Uri.tryParse(value);
  if (uri == null) return;

  final launched = await launchUrl(uri, mode: LaunchMode.externalApplication);
  if (!launched && context.mounted) {
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(const SnackBar(content: Text('无法打开原文链接')));
  }
}

class _ModeChip extends StatelessWidget {
  final String label;
  final bool selected;
  final VoidCallback onTap;

  const _ModeChip({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(999),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        decoration: BoxDecoration(
          color: selected
              ? AppTheme.primaryGreen.withValues(alpha: 0.16)
              : Colors.black26,
          borderRadius: BorderRadius.circular(999),
          border: Border.all(
            color: selected
                ? AppTheme.primaryGreen.withValues(alpha: 0.7)
                : Colors.white.withValues(alpha: 0.08),
          ),
        ),
        child: Text(
          label,
          style: TextStyle(
            color: selected ? AppTheme.primaryGreen : Colors.white70,
            fontWeight: FontWeight.w800,
            fontSize: 12,
          ),
        ),
      ),
    );
  }
}

class _ReferenceCard extends StatelessWidget {
  final LoopDraftReference reference;

  const _ReferenceCard({required this.reference});

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
            reference.title ?? '原内容',
            style: const TextStyle(fontWeight: FontWeight.w800),
          ),
          if (reference.quoteText != null &&
              reference.quoteText!.isNotEmpty) ...[
            const SizedBox(height: 6),
            Text(
              '“${reference.quoteText}”',
              style: const TextStyle(color: Colors.white60, height: 1.5),
            ),
          ],
          if (reference.sourceUrl != null &&
              reference.sourceUrl!.trim().isNotEmpty) ...[
            const SizedBox(height: 10),
            TextButton.icon(
              onPressed: () => _openLoopSourceUrl(context, reference.sourceUrl),
              style: TextButton.styleFrom(
                foregroundColor: AppTheme.textMuted,
                padding: EdgeInsets.zero,
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
              ),
              icon: const Icon(Icons.open_in_new_rounded, size: 16),
              label: const Text('原文链接'),
            ),
          ],
        ],
      ),
    );
  }
}

class _LoopPostCard extends StatelessWidget {
  final LoopPostEntry post;

  const _LoopPostCard({required this.post});

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
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              _MetaPill(text: _feedbackLabel(post.feedbackMode)),
              if (loopPreferenceStatusLabel(post.preferenceStatus) != null)
                _MetaPill(
                  text: loopPreferenceStatusLabel(post.preferenceStatus)!,
                ),
              if (post.createdAt != null)
                _MetaPill(text: _formatTime(post.createdAt!)),
            ],
          ),
          if (post.title != null && post.title!.isNotEmpty) ...[
            const SizedBox(height: 12),
            Text(
              post.title!,
              style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w900),
            ),
          ],
          const SizedBox(height: 10),
          Text(
            post.body,
            style: const TextStyle(height: 1.6, color: Colors.white70),
          ),
          if (post.references.isNotEmpty) ...[
            const SizedBox(height: 14),
            ...post.references.map(
              (reference) => _LoopPostReferenceCard(reference: reference),
            ),
          ],
        ],
      ),
    );
  }
}

class _LoopPostReferenceCard extends StatelessWidget {
  final LoopPostReference reference;

  const _LoopPostReferenceCard({required this.reference});

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.only(top: 8),
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: Colors.black26,
        borderRadius: BorderRadius.circular(14),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            reference.title ?? '原内容',
            style: const TextStyle(fontWeight: FontWeight.w800),
          ),
          if (reference.quoteText != null &&
              reference.quoteText!.isNotEmpty) ...[
            const SizedBox(height: 6),
            Text(
              '“${reference.quoteText}”',
              style: const TextStyle(color: Colors.white60, height: 1.5),
            ),
          ],
          if (reference.sourceUrl != null &&
              reference.sourceUrl!.trim().isNotEmpty) ...[
            const SizedBox(height: 10),
            TextButton.icon(
              onPressed: () => _openLoopSourceUrl(context, reference.sourceUrl),
              style: TextButton.styleFrom(
                foregroundColor: AppTheme.textMuted,
                padding: EdgeInsets.zero,
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
              ),
              icon: const Icon(Icons.open_in_new_rounded, size: 16),
              label: const Text('原文链接'),
            ),
          ],
        ],
      ),
    );
  }
}

class _MetaPill extends StatelessWidget {
  final String text;

  const _MetaPill({required this.text});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: Colors.white.withValues(alpha: 0.08),
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        text,
        style: const TextStyle(fontSize: 11, color: Colors.white70),
      ),
    );
  }
}

String _feedbackLabel(String mode) {
  switch (mode) {
    case 'boost':
      return '加重一点';
    case 'reduce':
      return '降一点';
    case 'observe':
      return '仅观察';
    default:
      return '平衡记录';
  }
}

String _formatTime(int ts) {
  final date = DateTime.fromMillisecondsSinceEpoch(ts * 1000);
  final month = date.month.toString().padLeft(2, '0');
  final day = date.day.toString().padLeft(2, '0');
  final hour = date.hour.toString().padLeft(2, '0');
  final minute = date.minute.toString().padLeft(2, '0');
  return '$month/$day $hour:$minute';
}
