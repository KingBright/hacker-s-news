import 'dart:convert';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:http/io_client.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:path_provider/path_provider.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:open_filex/open_filex.dart';
import 'package:shared_preferences/shared_preferences.dart';

class UpdateManager {
  static final FlutterLocalNotificationsPlugin _notificationsPlugin =
      FlutterLocalNotificationsPlugin();
  static const String _versionUrl = "https://news.hackerlife.fun:8443/version.json";
  static const Duration _resumeCheckInterval = Duration(hours: 6);
  static const String _lastCheckAtKey = 'freshloop_update_last_check_at_ms';
  static const String _lastNotifiedBuildKey =
      'freshloop_update_last_notified_build';
  static const String _pendingApkPathKey = 'freshloop_update_pending_apk_path';
  static const String _pendingVersionKey = 'freshloop_update_pending_version';
  static const String _pendingBuildKey = 'freshloop_update_pending_build';
  static bool _isChecking = false;

  static Future<void> initialize() async {
    const AndroidInitializationSettings initializationSettingsAndroid =
        AndroidInitializationSettings('@mipmap/ic_launcher');
    
    const InitializationSettings initializationSettings = InitializationSettings(
      android: initializationSettingsAndroid,
    );
    
    await _notificationsPlugin.initialize(
      settings: initializationSettings,
      onDidReceiveNotificationResponse: (NotificationResponse response) async {
        if (response.payload != null && response.payload!.endsWith('.apk')) {
          await OpenFilex.open(response.payload!);
        }
      },
    );

    await _ensureNotificationPermission(prompt: true);
    await _deliverPendingNotificationIfPossible();
  }

  static Future<void> checkAndDownloadUpdate({bool force = false}) async {
    if (_isChecking || !Platform.isAndroid) return;
    _isChecking = true;
    var didReceiveVersionResponse = false;

    try {
      await _deliverPendingNotificationIfPossible();

      if (!force && !await _shouldCheckNow()) {
        return;
      }

      // 1. Get current version
      final PackageInfo packageInfo = await PackageInfo.fromPlatform();
      final int currentBuildNumber = int.tryParse(packageInfo.buildNumber) ?? 0;

      // 2. Fetch remote version info
      final httpClient = HttpClient()
        ..badCertificateCallback =
            ((X509Certificate cert, String host, int port) => true);
      final ioClient = IOClient(httpClient);

      try {
        final response = await ioClient.get(Uri.parse(_versionUrl));
        didReceiveVersionResponse = true;
        if (response.statusCode != 200) return;

        final Map<String, dynamic> data = json.decode(response.body);
        final int remoteBuildNumber = data['build_number'] ?? 0;
        final String downloadUrl = data['download_url'] ?? '';
        final String versionName = data['version'] ?? '';

        // 3. Compare versions
        if (remoteBuildNumber > currentBuildNumber && downloadUrl.isNotEmpty) {
          debugPrint("Update available: $versionName ($remoteBuildNumber)");
          await _downloadAndNotify(downloadUrl, versionName, remoteBuildNumber);
        } else {
          debugPrint("App is up to date.");
        }
      } finally {
        ioClient.close();
        httpClient.close(force: true);
      }
    } catch (e) {
      debugPrint("Update check failed: $e");
    } finally {
      if (didReceiveVersionResponse) {
        await _markCheckedNow();
      }
      _isChecking = false;
    }
  }

  static Future<void> _downloadAndNotify(
    String url,
    String versionName,
    int remoteBuildNumber,
  ) async {
    try {
      // Get storage directory
      final Directory? dir = await getExternalStorageDirectory();
      if (dir == null) return;

      final String savePath = '${dir.path}/update_v$versionName.apk';
      final File file = File(savePath);
      final File tempFile = File('$savePath.part');

      final expectedBytes = await _fetchRemoteApkSize(url);

      if (await tempFile.exists()) {
        await tempFile.delete();
      }

      // If a previously completed APK is still intact, reuse it.
      if (await _isUsableDownloadedApk(file, expectedBytes)) {
        await _showOrQueueInstallNotification(
          savePath,
          versionName,
          remoteBuildNumber,
        );
        return;
      }

      if (await file.exists()) {
        await file.delete();
      }

      // Download silently
      final httpClient = HttpClient()
        ..badCertificateCallback =
            ((X509Certificate cert, String host, int port) => true);
      final ioClient = IOClient(httpClient);

      try {
        final request = http.Request('GET', Uri.parse(url));
        final http.StreamedResponse response = await ioClient.send(request);

        if (response.statusCode == 200) {
          var downloadedBytes = 0;
          final sink = tempFile.openWrite();

          try {
            await for (final chunk in response.stream) {
              downloadedBytes += chunk.length;
              sink.add(chunk);
            }
          } finally {
            await sink.close();
          }

          final responseLength = response.contentLength;
          final responseBytes =
              responseLength != null && responseLength > 0
                  ? responseLength
                  : expectedBytes;
          if (responseBytes != null && downloadedBytes != responseBytes) {
            debugPrint(
              "Downloaded APK size mismatch for v$versionName: expected $responseBytes bytes, got $downloadedBytes bytes.",
            );
            await tempFile.delete();
            return;
          }

          await tempFile.rename(savePath);

          // Notify user
          await _showOrQueueInstallNotification(
            savePath,
            versionName,
            remoteBuildNumber,
          );
        }
      } finally {
        if (await tempFile.exists()) {
          await tempFile.delete();
        }
        ioClient.close();
        httpClient.close(force: true);
      }
    } catch (e) {
      debugPrint("Download failed: $e");
    }
  }

  static Future<void> _showOrQueueInstallNotification(
    String apkPath,
    String versionName,
    int remoteBuildNumber,
  ) async {
    if (!await _ensureNotificationPermission(prompt: true)) {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString(_pendingApkPathKey, apkPath);
      await prefs.setString(_pendingVersionKey, versionName);
      await prefs.setInt(_pendingBuildKey, remoteBuildNumber);
      debugPrint(
        "Notification permission unavailable; queued update notice for v$versionName.",
      );
      return;
    }

    await _showInstallNotification(apkPath, versionName, remoteBuildNumber);
  }

  static Future<void> _showInstallNotification(
    String apkPath,
    String versionName,
    int remoteBuildNumber,
  ) async {
    const AndroidNotificationDetails androidPlatformChannelSpecifics =
        AndroidNotificationDetails(
      'update_channel',
      'App Updates',
      channelDescription: 'Notifications for app updates',
      importance: Importance.max,
      priority: Priority.high,
      autoCancel: true,
      playSound: true,
    );
    
    const NotificationDetails platformChannelSpecifics =
        NotificationDetails(android: androidPlatformChannelSpecifics);
        
    await _notificationsPlugin.show(
      id: 0,
      title: 'FreshLoop 新版本已就绪',
      body: '点击安装 v$versionName 更新',
      notificationDetails: platformChannelSpecifics,
      payload: apkPath,
    );

    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(_pendingApkPathKey);
    await prefs.remove(_pendingVersionKey);
    await prefs.remove(_pendingBuildKey);
    await prefs.setInt(_lastCheckAtKey, DateTime.now().millisecondsSinceEpoch);
    await prefs.setInt(_lastNotifiedBuildKey, remoteBuildNumber);
  }

  static Future<bool> _ensureNotificationPermission({
    required bool prompt,
  }) async {
    final androidImpl = _notificationsPlugin
        .resolvePlatformSpecificImplementation<
          AndroidFlutterLocalNotificationsPlugin
        >();
    if (androidImpl == null) return true;

    final bool? enabled = await androidImpl.areNotificationsEnabled();
    if (enabled ?? false) return true;
    if (!prompt) return false;

    final bool? granted = await androidImpl.requestNotificationsPermission();
    return granted ?? false;
  }

  static Future<void> _deliverPendingNotificationIfPossible() async {
    final prefs = await SharedPreferences.getInstance();
    final apkPath = prefs.getString(_pendingApkPathKey);
    final versionName = prefs.getString(_pendingVersionKey);
    final buildNumber = prefs.getInt(_pendingBuildKey);
    if (apkPath == null || versionName == null || buildNumber == null) return;

    final file = File(apkPath);
    if (!await file.exists()) {
      await prefs.remove(_pendingApkPathKey);
      await prefs.remove(_pendingVersionKey);
      await prefs.remove(_pendingBuildKey);
      return;
    }

    if (!await _ensureNotificationPermission(prompt: false)) {
      return;
    }

    await _showInstallNotification(apkPath, versionName, buildNumber);
  }

  static Future<int?> _fetchRemoteApkSize(String url) async {
    final httpClient = HttpClient()
      ..badCertificateCallback =
          ((X509Certificate cert, String host, int port) => true);
    final ioClient = IOClient(httpClient);

    try {
      final response = await ioClient.head(Uri.parse(url));
      final responseLength = response.contentLength;
      if (response.statusCode != 200 ||
          responseLength == null ||
          responseLength <= 0) {
        return null;
      }
      return responseLength;
    } catch (e) {
      debugPrint("Failed to fetch remote APK size: $e");
      return null;
    } finally {
      ioClient.close();
      httpClient.close(force: true);
    }
  }

  static Future<bool> _isUsableDownloadedApk(
    File file,
    int? expectedBytes,
  ) async {
    if (!await file.exists()) {
      return false;
    }

    final localBytes = await file.length();
    if (localBytes <= 0) {
      return false;
    }

    if (expectedBytes != null && localBytes != expectedBytes) {
      debugPrint(
        "Discarding stale APK because size mismatched: expected $expectedBytes bytes, got $localBytes bytes.",
      );
      return false;
    }

    return true;
  }

  static Future<bool> _shouldCheckNow() async {
    final prefs = await SharedPreferences.getInstance();
    final lastCheckedAt = prefs.getInt(_lastCheckAtKey) ?? 0;
    final now = DateTime.now().millisecondsSinceEpoch;
    return now - lastCheckedAt >= _resumeCheckInterval.inMilliseconds;
  }

  static Future<void> _markCheckedNow() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setInt(_lastCheckAtKey, DateTime.now().millisecondsSinceEpoch);
  }
}
