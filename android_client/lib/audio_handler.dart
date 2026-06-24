import 'dart:async';

import 'package:audio_service/audio_service.dart';
import 'package:audio_session/audio_session.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:just_audio/just_audio.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:android_client/src/rust/api/client.dart';
import 'package:android_client/src/rust/models.dart';

/// Callback invoked when a track finishes playing.
typedef OnTrackCompleted = FutureOr<void> Function(String itemId);

enum QueuePlaybackMode { dynamicContinuous, staticPlaylist }

class FreshLoopAudioHandler extends BaseAudioHandler
    with QueueHandler, SeekHandler {
  final _player = AudioPlayer();
  final FreshLoopClient client;
  final String baseUrl;
  static const int _maxAutomaticSkips = 3;
  static const Duration _bookmarkSaveInterval = Duration(seconds: 5);
  static const Duration _bookmarkMinPosition = Duration(seconds: 3);
  static const Duration _bookmarkNearEndThreshold = Duration(seconds: 10);
  static const Duration _bookmarkMaxAge = Duration(days: 14);
  static const String _bookmarkItemIdKey = 'freshloop_playback_item_id';
  static const String _bookmarkPositionKey = 'freshloop_playback_position_ms';
  static const String _bookmarkDurationKey = 'freshloop_playback_duration_ms';
  static const String _bookmarkUpdatedAtKey =
      'freshloop_playback_updated_at_ms';
  static const MediaControl _previousControl = MediaControl(
    androidIcon: 'drawable/fln_skip_previous',
    label: 'Previous',
    action: MediaAction.skipToPrevious,
  );
  static const MediaControl _rewindControl = MediaControl(
    androidIcon: 'drawable/fln_rewind',
    label: 'Rewind',
    action: MediaAction.rewind,
  );
  static const MediaControl _playControl = MediaControl(
    androidIcon: 'drawable/fln_play',
    label: 'Play',
    action: MediaAction.play,
  );
  static const MediaControl _pauseControl = MediaControl(
    androidIcon: 'drawable/fln_pause',
    label: 'Pause',
    action: MediaAction.pause,
  );
  static const MediaControl _fastForwardControl = MediaControl(
    androidIcon: 'drawable/fln_fast_forward',
    label: 'Fast Forward',
    action: MediaAction.fastForward,
  );
  static const MediaControl _nextControl = MediaControl(
    androidIcon: 'drawable/fln_skip_next',
    label: 'Next',
    action: MediaAction.skipToNext,
  );

  bool _isHandlingCompletion = false;
  bool _isLoadingItem = false;
  bool _interruptedWhilePlaying = false;
  bool _bookmarkStoreDisabled = false;
  int _currentQueueIndex = -1;
  String? _currentItemId;
  DateTime? _lastBookmarkSaveAt;
  SharedPreferences? _prefs;
  OnTrackCompleted? _queueCompletionHandler;
  QueuePlaybackMode _queuePlaybackMode = QueuePlaybackMode.dynamicContinuous;

  FreshLoopAudioHandler(this.client, this.baseUrl) {
    _init();
  }

  Future<void> _init() async {
    await _configureAudioSession();
    await _initBookmarkStore();

    Timer.periodic(_bookmarkSaveInterval, (_) {
      if (_player.playing) {
        _scheduleBookmarkSave();
      }
    });

    _player.playbackEventStream.listen(
      (PlaybackEvent event) {
        _broadcastPlaybackState();
      },
      onError: (Object error, StackTrace stackTrace) {
        debugPrint("Playback event error: $error");
        unawaited(_handlePlaybackFailure(error));
      },
    );

    _player.processingStateStream.listen((state) {
      if (state == ProcessingState.completed &&
          !_isHandlingCompletion &&
          !_isLoadingItem) {
        unawaited(_handleTrackCompleted());
      }
    });

    _player.durationStream.listen((duration) {
      final currentItem = mediaItem.value;
      if (currentItem != null && duration != null) {
        mediaItem.add(currentItem.copyWith(duration: duration));
      }
    });
  }

  Future<void> _initBookmarkStore() async {
    try {
      _prefs = await SharedPreferences.getInstance();
    } on MissingPluginException catch (e) {
      _disableBookmarkStore('missing plugin during init', e);
    } on PlatformException catch (e) {
      _disableBookmarkStore('platform failure during init', e);
    }
  }

  Future<void> _configureAudioSession() async {
    try {
      final session = await AudioSession.instance;
      await session.configure(const AudioSessionConfiguration.speech());

      session.becomingNoisyEventStream.listen((_) {
        unawaited(pause());
      });

      session.interruptionEventStream.listen((event) {
        if (event.begin) {
          _interruptedWhilePlaying = _player.playing;
          if (event.type == AudioInterruptionType.pause ||
              event.type == AudioInterruptionType.unknown) {
            unawaited(pause());
          } else if (event.type == AudioInterruptionType.duck) {
            unawaited(_player.setVolume(0.35));
          }
          return;
        }

        if (event.type == AudioInterruptionType.duck) {
          unawaited(_player.setVolume(1.0));
        } else if (_interruptedWhilePlaying &&
            event.type == AudioInterruptionType.pause) {
          _interruptedWhilePlaying = false;
          unawaited(play());
        } else {
          _interruptedWhilePlaying = false;
        }
      });
    } on MissingPluginException catch (e) {
      debugPrint("Audio session plugin unavailable: $e");
    } on PlatformException catch (e) {
      debugPrint("Failed to configure audio session: $e");
    }
  }

  void _broadcastPlaybackState() {
    final playing = _player.playing;
    playbackState.add(
      playbackState.value.copyWith(
        controls: [
          _previousControl,
          _rewindControl,
          if (playing) _pauseControl else _playControl,
          _fastForwardControl,
          _nextControl,
        ],
        systemActions: const {
          MediaAction.seek,
          MediaAction.seekForward,
          MediaAction.seekBackward,
          MediaAction.fastForward,
          MediaAction.rewind,
        },
        androidCompactActionIndices: const [0, 2, 4],
        processingState: _mapProcessingState(_player.processingState),
        playing: playing,
        updatePosition: _player.position,
        bufferedPosition: _player.bufferedPosition,
        speed: _player.speed,
        queueIndex: _currentQueueIndex >= 0 ? _currentQueueIndex : null,
      ),
    );
  }

  AudioProcessingState _mapProcessingState(ProcessingState state) {
    return switch (state) {
      ProcessingState.idle => AudioProcessingState.idle,
      ProcessingState.loading => AudioProcessingState.loading,
      ProcessingState.buffering => AudioProcessingState.buffering,
      ProcessingState.ready => AudioProcessingState.ready,
      ProcessingState.completed => AudioProcessingState.completed,
    };
  }

  Future<void> _handleTrackCompleted() async {
    if (_isHandlingCompletion) return;
    _isHandlingCompletion = true;

    try {
      final currentItem = mediaItem.value;
      if (currentItem == null) return;

      final completedId = currentItem.id;
      final nextItem = _nextItemAfter(completedId, queue.value);
      await _clearPlaybackBookmarkIfAvailable(itemId: completedId);

      final callback = _queueCompletionHandler;
      if (callback != null) {
        await callback(completedId);
      }

      final updatedQueue = queue.value;
      final target = nextItem != null && _containsId(updatedQueue, nextItem.id)
          ? nextItem
          : _queuePlaybackMode == QueuePlaybackMode.dynamicContinuous
          ? updatedQueue.where((item) => item.id != completedId).firstOrNull
          : null;

      if (target != null) {
        await _playMediaItem(target);
      } else {
        _currentItemId = null;
        _currentQueueIndex = -1;
        await _player.stop();
        _broadcastPlaybackState();
      }
    } finally {
      _isHandlingCompletion = false;
    }
  }

  Future<void> _handlePlaybackFailure(Object error) async {
    if (_isHandlingCompletion || _isLoadingItem) return;

    final failedId = _currentItemId ?? mediaItem.value?.id;
    if (failedId == null) return;

    await _skipPast(failedId, 0);
  }

  @override
  Future<void> rewind() async {
    final newPos = _player.position - const Duration(seconds: 15);
    await _player.seek(newPos < Duration.zero ? Duration.zero : newPos);
    await _savePlaybackBookmarkIfAvailable(force: true);
  }

  @override
  Future<void> fastForward() async {
    final newPos = _player.position + const Duration(seconds: 30);
    final dur = _player.duration ?? Duration.zero;
    await _player.seek(newPos > dur ? dur : newPos);
    await _savePlaybackBookmarkIfAvailable(force: true);
  }

  @override
  Future<void> play() async {
    try {
      await _startPlayer();
    } on PlayerInterruptedException catch (e) {
      debugPrint("Playback start interrupted: ${e.message}");
    } on PlayerException catch (e) {
      debugPrint("Playback start failed: ${e.code} ${e.message}");
      await _handlePlaybackFailure(e);
    }
  }

  @override
  Future<void> pause() async {
    await _player.pause();
    await _savePlaybackBookmarkIfAvailable(force: true);
  }

  @override
  Future<void> stop() async {
    await _savePlaybackBookmarkIfAvailable(force: true);
    _currentItemId = null;
    _currentQueueIndex = -1;
    await _player.stop();
    playbackState.add(
      playbackState.value.copyWith(
        processingState: AudioProcessingState.idle,
        playing: false,
        queueIndex: null,
      ),
    );
  }

  @override
  Future<void> seek(Duration position) async {
    await _player.seek(position);
    await _savePlaybackBookmarkIfAvailable(force: true, allowEarly: true);
  }

  @override
  Future<void> setSpeed(double speed) => _player.setSpeed(speed);

  @override
  Future<void> skipToQueueItem(int index) async {
    if (index < 0 || index >= queue.value.length) return;

    final mediaItemTarget = queue.value[index];
    await _playMediaItem(mediaItemTarget);
  }

  Future<void> _playMediaItem(
    MediaItem mediaItemTarget, {
    int automaticSkipCount = 0,
    Duration? initialPosition,
    bool playWhenReady = true,
    bool skipOnFailure = true,
  }) async {
    final audioUri = _audioUriFor(mediaItemTarget);
    if (audioUri == null) {
      debugPrint("Skipping item without audio URL: ${mediaItemTarget.id}");
      if (skipOnFailure) {
        await _skipPast(mediaItemTarget.id, automaticSkipCount);
      }
      return;
    }

    _isLoadingItem = true;
    _currentItemId = mediaItemTarget.id;
    _currentQueueIndex = _indexOfId(queue.value, mediaItemTarget.id);
    mediaItem.add(mediaItemTarget);
    _broadcastPlaybackState();

    var sourceReady = false;
    try {
      await _player.setAudioSource(
        AudioSource.uri(audioUri),
        initialPosition: initialPosition ?? Duration.zero,
      );
      sourceReady = true;
    } on PlayerInterruptedException catch (e) {
      debugPrint(
        "Loading item ${mediaItemTarget.id} was interrupted: ${e.message}",
      );
    } on PlayerException catch (e) {
      await _handleItemPlaybackFailure(
        mediaItemTarget,
        automaticSkipCount,
        skipOnFailure,
        '${e.code} ${e.message}',
      );
    } finally {
      _isLoadingItem = false;
      _broadcastPlaybackState();
    }

    if (sourceReady && playWhenReady) {
      await _startPlayer();
    }
  }

  Future<void> _skipPast(String itemId, int automaticSkipCount) async {
    if (automaticSkipCount >= _maxAutomaticSkips) {
      playbackState.add(
        playbackState.value.copyWith(
          processingState: AudioProcessingState.error,
          errorMessage: 'Too many consecutive playback failures',
        ),
      );
      return;
    }

    final next = _nextItemAfter(itemId, queue.value);
    if (next != null) {
      await _playMediaItem(next, automaticSkipCount: automaticSkipCount + 1);
    }
  }

  @override
  Future<void> skipToNext() async {
    final queueList = queue.value;
    final currentId = _currentItemId ?? mediaItem.value?.id;
    if (currentId == null) return;
    final currentIndex = _indexOfId(queueList, currentId);
    if (currentIndex < queueList.length - 1) {
      await skipToQueueItem(currentIndex + 1);
    }
  }

  @override
  Future<void> skipToPrevious() async {
    final queueList = queue.value;
    final currentId = _currentItemId ?? mediaItem.value?.id;
    if (currentId == null) return;
    final currentIndex = _indexOfId(queueList, currentId);
    if (currentIndex > 0) {
      await skipToQueueItem(currentIndex - 1);
    }
  }

  Future<void> updateQueueWithItems(
    List<Item> items, {
    OnTrackCompleted? onTrackCompleted,
    QueuePlaybackMode playbackMode = QueuePlaybackMode.dynamicContinuous,
  }) async {
    _queueCompletionHandler = onTrackCompleted;
    _queuePlaybackMode = playbackMode;

    final List<MediaItem> mediaItems = items
        .map(
          (item) => MediaItem(
            id: item.id,
            title: _cleanTitle(item),
            artist: _notificationSubtitle(item),
            album: 'FreshLoop Audio Briefing',
            genre: item.category,
            artUri: _artUriFor(item),
            duration: item.durationSec != null
                ? Duration(seconds: item.durationSec!.toInt())
                : null,
            displayTitle: _cleanTitle(item),
            displaySubtitle: _notificationSubtitle(item),
            displayDescription: _notificationDescription(item),
            extras: _extrasFor(item),
          ),
        )
        .toList();

    queue.add(mediaItems);
    if (_currentItemId != null) {
      _currentQueueIndex = _indexOfId(mediaItems, _currentItemId!);
      _broadcastPlaybackState();
    } else {
      await _restorePlaybackBookmark(mediaItems);
    }
  }

  Future<void> _restorePlaybackBookmark(List<MediaItem> mediaItems) async {
    if (_isLoadingItem || mediaItems.isEmpty) return;

    final prefs = await _bookmarkPrefs();
    if (prefs == null) return;

    final itemId = prefs.getString(_bookmarkItemIdKey);
    if (itemId == null || itemId.isEmpty) return;

    final updatedAtMs = prefs.getInt(_bookmarkUpdatedAtKey) ?? 0;
    if (updatedAtMs > 0) {
      final age = DateTime.now().difference(
        DateTime.fromMillisecondsSinceEpoch(updatedAtMs),
      );
      if (age > _bookmarkMaxAge) {
        await _clearPlaybackBookmarkIfAvailable();
        return;
      }
    }

    final item = mediaItems
        .where((mediaItem) => mediaItem.id == itemId)
        .firstOrNull;
    if (item == null) return;

    final savedPosition = Duration(
      milliseconds: prefs.getInt(_bookmarkPositionKey) ?? 0,
    );
    final savedDurationMs = prefs.getInt(_bookmarkDurationKey);
    final duration =
        item.duration ??
        (savedDurationMs != null
            ? Duration(milliseconds: savedDurationMs)
            : null);

    if (_isNearEnd(savedPosition, duration)) {
      await _clearPlaybackBookmarkIfAvailable(itemId: itemId);
      return;
    }

    await _playMediaItem(
      item,
      initialPosition: savedPosition,
      playWhenReady: false,
      skipOnFailure: false,
    );
  }

  Future<void> _savePlaybackBookmark({
    bool force = false,
    bool allowEarly = false,
  }) async {
    final itemId = _currentItemId ?? mediaItem.value?.id;
    if (itemId == null) return;

    final now = DateTime.now();
    if (!force &&
        _lastBookmarkSaveAt != null &&
        now.difference(_lastBookmarkSaveAt!) < _bookmarkSaveInterval) {
      return;
    }

    final position = _player.position;
    final duration = mediaItem.value?.duration ?? _player.duration;
    if (!allowEarly && position < _bookmarkMinPosition) return;

    if (_isNearEnd(position, duration)) {
      await _clearPlaybackBookmarkIfAvailable(itemId: itemId);
      return;
    }

    final prefs = await _bookmarkPrefs();
    if (prefs == null) return;

    await prefs.setString(_bookmarkItemIdKey, itemId);
    await prefs.setInt(_bookmarkPositionKey, position.inMilliseconds);
    if (duration != null) {
      await prefs.setInt(_bookmarkDurationKey, duration.inMilliseconds);
    }
    await prefs.setInt(_bookmarkUpdatedAtKey, now.millisecondsSinceEpoch);
    _lastBookmarkSaveAt = now;
  }

  void _scheduleBookmarkSave({bool force = false, bool allowEarly = false}) {
    unawaited(
      _savePlaybackBookmarkIfAvailable(force: force, allowEarly: allowEarly),
    );
  }

  Future<void> _savePlaybackBookmarkIfAvailable({
    bool force = false,
    bool allowEarly = false,
  }) async {
    try {
      await _savePlaybackBookmark(force: force, allowEarly: allowEarly);
    } on MissingPluginException catch (e) {
      _disableBookmarkStore('missing plugin while saving', e);
    } on PlatformException catch (e) {
      _disableBookmarkStore('platform failure while saving', e);
    }
  }

  Future<void> _clearPlaybackBookmark({String? itemId}) async {
    final prefs = await _bookmarkPrefs();
    if (prefs == null) return;

    if (itemId != null && prefs.getString(_bookmarkItemIdKey) != itemId) {
      return;
    }

    await prefs.remove(_bookmarkItemIdKey);
    await prefs.remove(_bookmarkPositionKey);
    await prefs.remove(_bookmarkDurationKey);
    await prefs.remove(_bookmarkUpdatedAtKey);
  }

  Future<void> _clearPlaybackBookmarkIfAvailable({String? itemId}) async {
    try {
      await _clearPlaybackBookmark(itemId: itemId);
    } on MissingPluginException catch (e) {
      _disableBookmarkStore('missing plugin while clearing', e);
    } on PlatformException catch (e) {
      _disableBookmarkStore('platform failure while clearing', e);
    }
  }

  Future<SharedPreferences?> _bookmarkPrefs() async {
    if (_bookmarkStoreDisabled) return null;

    final cached = _prefs;
    if (cached != null) return cached;

    try {
      return _prefs = await SharedPreferences.getInstance();
    } on MissingPluginException catch (e) {
      _disableBookmarkStore('missing plugin while opening', e);
      return null;
    } on PlatformException catch (e) {
      _disableBookmarkStore('platform failure while opening', e);
      return null;
    }
  }

  void _disableBookmarkStore(String reason, Object error) {
    if (!_bookmarkStoreDisabled) {
      debugPrint("Playback bookmark store disabled ($reason): $error");
    }
    _bookmarkStoreDisabled = true;
    _prefs = null;
  }

  Future<void> _startPlayer() async {
    try {
      final playFuture = _player.play();
      unawaited(
        playFuture.catchError((Object error, StackTrace stackTrace) {
          if (error is PlayerInterruptedException) {
            debugPrint("Playback start interrupted: ${error.message}");
            return;
          }
          if (error is PlayerException) {
            debugPrint("Playback failed: ${error.code} ${error.message}");
          } else {
            debugPrint("Playback failed: $error");
          }
          unawaited(_handlePlaybackFailure(error));
        }),
      );
    } on PlayerInterruptedException catch (e) {
      debugPrint("Playback start interrupted: ${e.message}");
    } on PlayerException catch (e) {
      debugPrint("Playback start failed: ${e.code} ${e.message}");
      await _handlePlaybackFailure(e);
    }
  }

  Future<void> _handleItemPlaybackFailure(
    MediaItem failedItem,
    int automaticSkipCount,
    bool skipOnFailure,
    String reason,
  ) async {
    debugPrint("Failed to load item ${failedItem.id}: $reason");
    if (skipOnFailure) {
      await _skipPast(failedItem.id, automaticSkipCount);
    } else {
      await _clearPlaybackBookmarkIfAvailable(itemId: failedItem.id);
    }
  }

  bool _isNearEnd(Duration position, Duration? duration) {
    if (duration == null || duration == Duration.zero) return false;
    if (position >= duration) return true;
    return duration - position <= _bookmarkNearEndThreshold;
  }

  int _indexOfId(List<MediaItem> items, String itemId) {
    return items.indexWhere((item) => item.id == itemId);
  }

  bool _containsId(List<MediaItem> items, String itemId) {
    return _indexOfId(items, itemId) >= 0;
  }

  MediaItem? _nextItemAfter(String itemId, List<MediaItem> items) {
    final index = _indexOfId(items, itemId);
    if (index >= 0 && index < items.length - 1) {
      return items[index + 1];
    }
    return null;
  }

  Uri _artUriFor(Item item) {
    final coverUrl = item.coverImageUrl;
    if (coverUrl != null && coverUrl.isNotEmpty) {
      return Uri.parse(
        coverUrl.startsWith('http') ? coverUrl : '$baseUrl$coverUrl',
      );
    }
    return Uri.parse('$baseUrl/icon.png');
  }

  Uri? _audioUriFor(MediaItem item) {
    final rawAudioUrl = item.extras?['audio_url'];
    if (rawAudioUrl is! String) return null;

    final trimmed = rawAudioUrl.trim();
    if (trimmed.isEmpty) return null;

    final absoluteUri = Uri.tryParse(trimmed);
    if (absoluteUri != null && _isHttpUri(absoluteUri)) {
      return absoluteUri;
    }

    final baseUri = Uri.tryParse(baseUrl);
    if (baseUri == null || !_isHttpUri(baseUri)) return null;

    final resolvedUri = baseUri.resolve(trimmed);
    return _isHttpUri(resolvedUri) ? resolvedUri : null;
  }

  bool _isHttpUri(Uri uri) {
    final scheme = uri.scheme.toLowerCase();
    return scheme == 'http' || scheme == 'https';
  }

  String _cleanTitle(Item item) {
    return item.title.replaceAll(RegExp(r'^【.*?】'), '').trim();
  }

  String _notificationSubtitle(Item item) {
    final category = item.category?.trim();
    final duration = item.durationSec;
    final durationLabel = duration != null
        ? '${(duration ~/ 60)}:${(duration % 60).toInt().toString().padLeft(2, '0')}'
        : 'Brief';

    if (category != null && category.isNotEmpty) {
      return '$category · $durationLabel · FreshLoop';
    }
    return '$durationLabel · FreshLoop';
  }

  String _notificationDescription(Item item) {
    final summary = item.summary?.trim();
    if (summary != null && summary.isNotEmpty) {
      return summary.length > 140 ? '${summary.substring(0, 140)}...' : summary;
    }
    return 'FreshLoop audio briefing';
  }

  Map<String, dynamic> _extrasFor(Item item) {
    final extras = <String, dynamic>{};

    final audioUrl = item.audioUrl;
    if (audioUrl != null && audioUrl.isNotEmpty) {
      extras['audio_url'] = audioUrl;
    }

    final summary = item.summary?.trim();
    if (summary != null && summary.isNotEmpty) {
      extras['summary'] = summary;
    }

    final category = item.category?.trim();
    if (category != null && category.isNotEmpty) {
      extras['category'] = category;
    }

    final originalUrl = item.originalUrl?.trim();
    if (originalUrl != null && originalUrl.isNotEmpty) {
      extras['original_url'] = originalUrl;
    }

    return extras;
  }
}
