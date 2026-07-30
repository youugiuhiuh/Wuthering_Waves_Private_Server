import 'package:flutter/services.dart';

class NotificationService {
  static const _channel = MethodChannel('com.example.sni_tester/foreground');
  static const _permChannel = MethodChannel('com.example.sni_tester/permission');

  static Future<void> start() async {
    try {
      await _permChannel.invokeMethod('requestNotification');
      await _channel.invokeMethod('startForeground');
    } catch (_) {}
  }

  static Future<void> updateProgress(int done, int total) async {
    try {
      final status = '已完成 $done/$total (${total > 0 ? (done * 100 ~/ total) : 0}%)';
      await _channel.invokeMethod('updateProgress', {'status': status});
    } catch (_) {}
  }

  static Future<void> complete(int success, int failed) async {
    try {
      final status = '完成: $success 成功, $failed 失败';
      await _channel.invokeMethod('stopForeground', {'finalStatus': status});
    } catch (_) {}
  }
}
