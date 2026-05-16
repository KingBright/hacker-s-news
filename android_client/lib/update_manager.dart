import 'dart:convert';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:http/io_client.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:path_provider/path_provider.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:open_filex/open_filex.dart';

class UpdateManager {
  static final FlutterLocalNotificationsPlugin _notificationsPlugin = FlutterLocalNotificationsPlugin();
  static const String _versionUrl = "https://news.hackerlife.fun:8443/version.json";
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

    // Request permissions for Android 13+
    _notificationsPlugin.resolvePlatformSpecificImplementation<
        AndroidFlutterLocalNotificationsPlugin>()?.requestNotificationsPermission();
  }

  static Future<void> checkAndDownloadUpdate() async {
    if (_isChecking || !Platform.isAndroid) return;
    _isChecking = true;

    try {
      // 1. Get current version
      final PackageInfo packageInfo = await PackageInfo.fromPlatform();
      final int currentBuildNumber = int.tryParse(packageInfo.buildNumber) ?? 0;

      // 2. Fetch remote version info
      final httpClient = HttpClient()..badCertificateCallback = ((X509Certificate cert, String host, int port) => true);
      final ioClient = IOClient(httpClient);
      
      final response = await ioClient.get(Uri.parse(_versionUrl));
      if (response.statusCode != 200) return;

      final Map<String, dynamic> data = json.decode(response.body);
      final int remoteBuildNumber = data['build_number'] ?? 0;
      final String downloadUrl = data['download_url'] ?? '';
      final String versionName = data['version'] ?? '';

      // 3. Compare versions
      if (remoteBuildNumber > currentBuildNumber && downloadUrl.isNotEmpty) {
        debugPrint("Update available: $versionName ($remoteBuildNumber)");
        await _downloadAndNotify(downloadUrl, versionName);
      } else {
        debugPrint("App is up to date.");
      }
    } catch (e) {
      debugPrint("Update check failed: $e");
    } finally {
      _isChecking = false;
    }
  }

  static Future<void> _downloadAndNotify(String url, String versionName) async {
    try {
      // Get storage directory
      final Directory? dir = await getExternalStorageDirectory();
      if (dir == null) return;
      
      final String savePath = '${dir.path}/update_v$versionName.apk';
      final File file = File(savePath);

      // If already downloaded, just show notification
      if (await file.exists()) {
        await _showInstallNotification(savePath, versionName);
        return;
      }

      // Download silently
      final httpClient = HttpClient()..badCertificateCallback = ((X509Certificate cert, String host, int port) => true);
      final ioClient = IOClient(httpClient);
      
      final request = http.Request('GET', Uri.parse(url));
      final http.StreamedResponse response = await ioClient.send(request);
      
      if (response.statusCode == 200) {
        final sink = file.openWrite();
        await response.stream.pipe(sink);
        await sink.close();
        
        // Notify user
        await _showInstallNotification(savePath, versionName);
      }
    } catch (e) {
      debugPrint("Download failed: $e");
    }
  }

  static Future<void> _showInstallNotification(String apkPath, String versionName) async {
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
  }
}
