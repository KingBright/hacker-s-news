import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:audio_service/audio_service.dart';
import 'package:android_client/src/rust/api/client.dart';
import 'package:android_client/src/rust/models.dart';
import 'package:android_client/src/rust/frb_generated.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'audio_handler.dart';
import 'ui/theme.dart';
import 'ui/feed_screen.dart';
import 'update_manager.dart';
import 'dart:async';

late FreshLoopAudioHandler audioHandler;
final String baseUrl = 'https://news.hackerlife.fun:8443';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  try {
    await UpdateManager.initialize();
  } catch (e) {
    debugPrint("Failed to initialize UpdateManager: \$e");
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

    // Check for updates
    UpdateManager.checkAndDownloadUpdate();

    runApp(
      MultiProvider(
        providers: [
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
class FreshLoopApp extends StatelessWidget {
  const FreshLoopApp({super.key});

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
  final Set<String> _playedIds = {};
  bool _isBackfilling = false;

  FeedProvider(this.client) {
    // Wire up the track-completed callback
    audioHandler.onTrackCompleted = _onTrackCompleted;
    fetchItems();
  }

  Future<void> _onTrackCompleted(String itemId) async {
    _playedIds.add(itemId);
    items.removeWhere((item) => item.id == itemId);
    notifyListeners();

    await audioHandler.updateQueueWithItems(items);

    if (items.length < maxQueueSize) {
      await _backfill();
    }
  }

  void markAsPlayed(String itemId) {
    client.markAsPlayed(id: itemId).catchError((e) {
      debugPrint("Failed to mark as played manually: $e");
    });

    _playedIds.add(itemId);
    items.removeWhere((item) => item.id == itemId);
    notifyListeners();

    unawaited(_syncQueueAndBackfill());
  }

  Future<void> _syncQueueAndBackfill() async {
    await audioHandler.updateQueueWithItems(items);
    if (items.length < maxQueueSize) {
      await _backfill();
    }
  }

  Future<void> _backfill() async {
    if (_isBackfilling || isLoading) return;
    _isBackfilling = true;

    try {
      final newItems = await client.fetchItems(
        page: page,
        limit: maxQueueSize - items.length,
      );
      if (newItems.isEmpty) return;
      page++;

      final existingIds = items.map((i) => i.id).toSet();
      final fresh = newItems
          .where(
            (i) => !existingIds.contains(i.id) && !_playedIds.contains(i.id),
          )
          .toList();

      if (fresh.isNotEmpty) {
        items.addAll(fresh);
        // Backend returns DESC (newest first). We sort ASC (oldest first) so playback is chronological.
        items.sort(
          (a, b) => (a.publishTime ?? 0).compareTo(b.publishTime ?? 0),
        );
        await audioHandler.updateQueueWithItems(items);
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
      final newItems = await client.fetchItems(page: page, limit: maxQueueSize);
      items.addAll(newItems);
      // Remove duplicates and played items
      final seen = <String>{};
      items.retainWhere(
        (item) => seen.add(item.id) && !_playedIds.contains(item.id),
      );

      // Backend returns DESC (newest first). We sort ASC (oldest first) so playback is chronological.
      items.sort((a, b) => (a.publishTime ?? 0).compareTo(b.publishTime ?? 0));
      page++;
      await audioHandler.updateQueueWithItems(items);
    } catch (e) {
      debugPrint("Error fetching items: $e");
    } finally {
      isLoading = false;
      notifyListeners();
    }
  }
}
