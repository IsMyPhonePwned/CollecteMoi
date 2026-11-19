import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Native bridges for share intake and Android settings deep-links.
class PlatformBridge {
  static const _channel = MethodChannel('collectemoi/native');
  static const _events = EventChannel('collectemoi/share_events');

  Stream<Map<String, dynamic>> get shareEvents =>
      _events.receiveBroadcastStream().map((e) {
        if (e is Map) {
          return e.map((k, v) => MapEntry(k.toString(), v));
        }
        return <String, dynamic>{'path': e.toString()};
      });

  Future<bool> isDeveloperOptionsEnabled() async {
    if (defaultTargetPlatform != TargetPlatform.android) return false;
    try {
      final result =
          await _channel.invokeMethod<bool>('isDeveloperOptionsEnabled');
      return result ?? false;
    } on MissingPluginException {
      return false;
    }
  }

  Future<void> openDeviceInfoSettings() async {
    if (defaultTargetPlatform != TargetPlatform.android) return;
    try {
      await _channel.invokeMethod('openDeviceInfoSettings');
    } on MissingPluginException {
      // no-op on unsupported platforms
    }
  }

  Future<void> openDeveloperSettings() async {
    if (defaultTargetPlatform != TargetPlatform.android) return;
    try {
      await _channel.invokeMethod('openDeveloperSettings');
    } on MissingPluginException {
      // no-op
    }
  }

  Future<List<Map<String, dynamic>>> pendingShareJobs() async {
    try {
      final result = await _channel.invokeMethod<List<dynamic>>('pendingShareJobs');
      return (result ?? [])
          .whereType<Map>()
          .map((e) => e.map((k, v) => MapEntry(k.toString(), v)))
          .toList();
    } on MissingPluginException {
      return [];
    }
  }

  Future<String?> importFromFiles() async {
    try {
      return await _channel.invokeMethod<String>('importFromFiles');
    } on MissingPluginException {
      return null;
    }
  }
}
