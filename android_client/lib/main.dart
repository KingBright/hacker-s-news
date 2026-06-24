import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:audio_service/audio_service.dart';
import 'package:android_client/src/rust/api/client.dart';
import 'package:android_client/src/rust/models.dart';
import 'package:android_client/src/rust/frb_generated.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'app_shell.dart';
import 'audio_handler.dart';
import 'day_playlist.dart';
import 'feed_api.dart';
import 'ui/theme.dart';
import 'ui/feed_screen.dart';
import 'ui/reading_screen.dart';
import 'update_manager.dart';
import 'dart:async';

late FreshLoopAudioHandler audioHandler;
final String baseUrl = 'https://news.hackerlife.fun:8443';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  try {
    await UpdateManager.initialize();
  } catch (e) {
    debugPrint("Failed to initialize UpdateManager: $e");
  }

  try {
    await RustLib.init();

    final prefs = await SharedPreferences.getInstance();
    final cachedUserId = prefs.getString('freshloop_user_id');
    final cachedUsername = prefs.getString('freshloop_username');

    final client = FreshLoopClient(baseUrl: baseUrl, userId: cachedUserId);

    audioHandler = await AudioService.init(
      builder: () => FreshLoopAudioHandler(client, baseUrl),
      config: const AudioServiceConfig(
        androidNotificationChannelId: 'fun.hackerlife.freshloop.playback',
        androidNotificationChannelName: 'FreshLoop Playback',
        androidNotificationChannelDescription:
            'Beautiful background playback controls for FreshLoop briefings',
        androidNotificationIcon: 'drawable/ic_stat_freshloop',
        notificationColor: AppTheme.primaryGreen,
        androidNotificationOngoing: false,
        androidStopForegroundOnPause: false,
        androidShowNotificationBadge: false,
        androidResumeOnClick: true,
        preloadArtwork: false,
        artDownscaleWidth: 512,
        artDownscaleHeight: 512,
        fastForwardInterval: Duration(seconds: 30),
        rewindInterval: Duration(seconds: 15),
      ),
    );

    User? initialUser;
    if (cachedUserId != null && cachedUsername != null) {
      initialUser = User(id: cachedUserId, username: cachedUsername);
    }

    runApp(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ShellProvider()),
          ChangeNotifierProvider(
            create: (_) => AuthProvider(client, initialUser),
          ),
          ChangeNotifierProxyProvider<AuthProvider, FeedProvider>(
            create: (context) => FeedProvider(client),
            update: (context, auth, feed) {
              if (feed != null && feed.userId != auth.user?.id) {
                feed.userId = auth.user?.id;
                feed.refresh();
              }
              return feed!;
            },
          ),
          ChangeNotifierProxyProvider<AuthProvider, ReadingFeedProvider>(
            create: (_) =>
                ReadingFeedProvider(CuratedFeedApi(baseUrl: baseUrl)),
            update: (context, auth, reading) {
              if (reading != null && reading.userId != auth.user?.id) {
                reading.userId = auth.user?.id;
                reading.refresh();
              }
              return reading!;
            },
          ),
        ],
        child: const FreshLoopApp(),
      ),
    );
  } catch (e, stacktrace) {
    debugPrint('App Initialization Failed: $e\n$stacktrace');
    runApp(ErrorApp(error: e.toString()));
  }
}

class AuthProvider extends ChangeNotifier {
  final FreshLoopClient _client;
  User? _user;

  AuthProvider(this._client, this._user);

  User? get user => _user;
  bool get isAuthenticated => _user != null;

  Future<void> login(String username, String password) async {
    final u = await _client.login(username: username, password: password);
    _user = u;

    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('freshloop_user_id', u.id);
    await prefs.setString('freshloop_username', u.username);

    notifyListeners();
  }

  Future<void> logout() async {
    _client.setUserId(userId: null);
    _user = null;

    final prefs = await SharedPreferences.getInstance();
    await prefs.remove('freshloop_user_id');
    await prefs.remove('freshloop_username');

    notifyListeners();
  }
}

class ErrorApp extends StatelessWidget {
  final String error;
  const ErrorApp({super.key, required this.error});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      theme: ThemeData.dark().copyWith(
        scaffoldBackgroundColor: const Color(0xFF111111),
      ),
      home: Scaffold(
        body: Center(
          child: Padding(
            padding: const EdgeInsets.all(24.0),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                const Icon(
                  Icons.error_outline,
                  color: Colors.redAccent,
                  size: 64,
                ),
                const SizedBox(height: 16),
                const Text(
                  'App Failed to Load',
                  style: TextStyle(fontSize: 20, fontWeight: FontWeight.bold),
                ),
                const SizedBox(height: 12),
                Text(
                  'A native error occurred:\n$error',
                  style: const TextStyle(color: Colors.white70),
                  textAlign: TextAlign.center,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

// App Entry Point and Providers
class FreshLoopApp extends StatefulWidget {
  const FreshLoopApp({super.key});

  @override
  State<FreshLoopApp> createState() => _FreshLoopAppState();
}

class _FreshLoopAppState extends State<FreshLoopApp>
    with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(UpdateManager.checkAndDownloadUpdate(force: true));
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      unawaited(UpdateManager.checkAndDownloadUpdate());
    }
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'FreshLoop',
      theme: AppTheme.darkTheme,
      home: const FeedScreen(),
      debugShowCheckedModeBanner: false,
    );
  }
}

class FeedProvider extends ChangeNotifier {
  final FreshLoopClient client;
  List<Item> items = [];
  bool isLoading = false;
  int page = 1;
  String? userId;
  static const int maxQueueSize = 50;
  static const String _pendingPlayedIdsKey = 'freshloop_pending_played_ids';
  final Set<String> _playedIds = {};
  final Set<String> _pendingPlayedIds = {};
  bool _isBackfilling = false;
  bool _isSyncingPlayed = false;
  bool _hasLoadedPendingPlayed = false;

  FeedProvider(this.client) {
    unawaited(_initialize());
  }

  List<DayPlaylistGroup<Item>> get dayGroups => buildDayPlaylists<Item>(
    items: items,
    idOf: (item) => item.id,
    timestampSecondsOf: (item) =>
        item.publishTime?.toInt() ?? item.createdAt?.toInt(),
    isPlayable: (item) => (item.audioUrl?.trim().isNotEmpty ?? false),
    durationSecondsOf: (item) => item.durationSec?.toInt(),
    dayOrder: DayPlaylistSortOrder.descending,
    itemOrder: DayPlaylistSortOrder.ascending,
    playbackOrder: DayPlaylistSortOrder.ascending,
  );

  Future<void> _initialize() async {
    await _loadPendingPlayedIds();
    await _syncPendingPlayedIds();
    await fetchItems();
  }

  Future<void> _loadPendingPlayedIds() async {
    if (_hasLoadedPendingPlayed) return;

    final prefs = await SharedPreferences.getInstance();
    final ids = prefs.getStringList(_pendingPlayedIdsKey) ?? const <String>[];
    _pendingPlayedIds.addAll(ids);
    _playedIds.addAll(ids);
    _hasLoadedPendingPlayed = true;
  }

  Future<void> _persistPendingPlayedIds() async {
    final prefs = await SharedPreferences.getInstance();
    final ids = _pendingPlayedIds.toList()..sort();
    await prefs.setStringList(_pendingPlayedIdsKey, ids);
  }

  Future<void> _rememberPlayed(String itemId) async {
    _playedIds.add(itemId);
    if (_pendingPlayedIds.add(itemId)) {
      await _persistPendingPlayedIds();
    }
    unawaited(_syncPendingPlayedIds());
  }

  Future<void> _syncPendingPlayedIds() async {
    if (_isSyncingPlayed) return;
    await _loadPendingPlayedIds();
    if (_pendingPlayedIds.isEmpty) return;

    _isSyncingPlayed = true;
    var changed = false;
    try {
      for (final id in List<String>.from(_pendingPlayedIds)) {
        try {
          await client.markAsPlayed(id: id);
          changed = _pendingPlayedIds.remove(id) || changed;
        } catch (e) {
          debugPrint("Will retry mark-as-played for $id later: $e");
          break;
        }
      }

      if (changed) {
        await _persistPendingPlayedIds();
      }
    } finally {
      _isSyncingPlayed = false;
    }
  }

  Future<void> _onTrackCompleted(String itemId) async {
    await _rememberPlayed(itemId);
    items.removeWhere((item) => item.id == itemId);
    notifyListeners();

    await audioHandler.updateQueueWithItems(
      items,
      onTrackCompleted: _onTrackCompleted,
      playbackMode: QueuePlaybackMode.dynamicContinuous,
    );

    if (items.length < maxQueueSize) {
      await _backfill();
    }
  }

  Future<void> markAsPlayed(String itemId) async {
    await _rememberPlayed(itemId);
    items.removeWhere((item) => item.id == itemId);
    notifyListeners();

    unawaited(_syncQueueAndBackfill());
  }

  Future<void> _syncQueueAndBackfill() async {
    await audioHandler.updateQueueWithItems(
      items,
      onTrackCompleted: _onTrackCompleted,
      playbackMode: QueuePlaybackMode.dynamicContinuous,
    );
    if (items.length < maxQueueSize) {
      await _backfill();
    }
  }

  Future<void> _backfill() async {
    if (_isBackfilling || isLoading) return;
    _isBackfilling = true;

    try {
      await _syncPendingPlayedIds();
      final newItems = await client.fetchItems(page: 1, limit: maxQueueSize);
      if (newItems.isEmpty) return;

      final existingIds = items.map((i) => i.id).toSet();
      final fresh = newItems
          .where(
            (i) => !existingIds.contains(i.id) && !_playedIds.contains(i.id),
          )
          .toList();

      if (fresh.isNotEmpty) {
        items.addAll(fresh);
        _normalizeQueue(trimToMax: true);
        await audioHandler.updateQueueWithItems(
          items,
          onTrackCompleted: _onTrackCompleted,
          playbackMode: QueuePlaybackMode.dynamicContinuous,
        );
        notifyListeners();
      }
    } catch (e) {
      debugPrint("Backfill error: $e");
    } finally {
      _isBackfilling = false;
    }
  }

  void refresh() {
    page = 1;
    items.clear();
    fetchItems();
  }

  Future<void> fetchItems() async {
    if (isLoading) return;
    isLoading = true;
    notifyListeners();

    try {
      await _loadPendingPlayedIds();
      await _syncPendingPlayedIds();
      final requestedPage = page;
      final newItems = await client.fetchItems(
        page: requestedPage,
        limit: maxQueueSize,
      );
      items.addAll(newItems);
      _normalizeQueue();
      page = requestedPage + 1;
      await audioHandler.updateQueueWithItems(
        items,
        onTrackCompleted: _onTrackCompleted,
        playbackMode: QueuePlaybackMode.dynamicContinuous,
      );
    } catch (e) {
      debugPrint("Error fetching items: $e");
    } finally {
      isLoading = false;
      notifyListeners();
    }
  }

  void _normalizeQueue({bool trimToMax = false}) {
    final seen = <String>{};
    items.retainWhere(
      (item) => seen.add(item.id) && !_playedIds.contains(item.id),
    );
    items.sort(_compareItemsByPlaybackOrder);
    if (trimToMax && items.length > maxQueueSize) {
      items.removeRange(maxQueueSize, items.length);
    }
  }

  int _compareItemsByPlaybackOrder(Item a, Item b) {
    final byTime = _queueTime(a).compareTo(_queueTime(b));
    if (byTime != 0) return byTime;
    return a.id.compareTo(b.id);
  }

  int _queueTime(Item item) => item.publishTime ?? item.createdAt ?? 0;

  Future<void> playDay(List<Item> dayItems, {String? startItemId}) async {
    final queueItems =
        dayItems
            .where((item) => item.audioUrl?.trim().isNotEmpty ?? false)
            .toList()
          ..sort(_compareItemsByPlaybackOrder);
    if (queueItems.isEmpty) return;

    final startIndex = startItemId == null
        ? 0
        : queueItems.indexWhere((item) => item.id == startItemId);
    final safeIndex = startIndex >= 0 ? startIndex : 0;

    await audioHandler.updateQueueWithItems(
      queueItems,
      onTrackCompleted: _onDayPlaylistTrackCompleted,
      playbackMode: QueuePlaybackMode.staticPlaylist,
    );
    await audioHandler.skipToQueueItem(safeIndex);
    await audioHandler.play();
  }

  Future<void> playWholeQueue({String? startItemId}) async {
    if (items.isEmpty) return;
    final queueItems = [...items]..sort(_compareItemsByPlaybackOrder);
    final startIndex = startItemId == null
        ? 0
        : queueItems.indexWhere((item) => item.id == startItemId);
    final safeIndex = startIndex >= 0 ? startIndex : 0;

    await audioHandler.updateQueueWithItems(
      queueItems,
      onTrackCompleted: _onTrackCompleted,
      playbackMode: QueuePlaybackMode.dynamicContinuous,
    );
    await audioHandler.skipToQueueItem(safeIndex);
    await audioHandler.play();
  }

  Future<void> _onDayPlaylistTrackCompleted(String itemId) async {
    await _rememberPlayed(itemId);
    items.removeWhere((item) => item.id == itemId);
    notifyListeners();

    if (items.length < maxQueueSize) {
      unawaited(_backfill());
    }
  }
}
