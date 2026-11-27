import 'dart:async';

import 'package:flutter/widgets.dart';

import 'api_client.dart';

enum ServerReachability { unknown, checking, online, offline }

/// Polls `GET /healthz` so the UI can show a live online / offline indicator.
///
/// Interval defaults to 60s and follows the lab setting advertised in
/// `x-collectemoi-healthz-interval` (Admin → Settings → Healthz poll).
class ServerHealth extends ChangeNotifier with WidgetsBindingObserver {
  ServerHealth(this._api) {
    WidgetsBinding.instance.addObserver(this);
    _restartTimer();
    unawaited(check());
  }

  static const int defaultIntervalSecs = 60;
  static const int minIntervalSecs = 15;
  static const int maxIntervalSecs = 3600;

  final ApiClient _api;
  Timer? _timer;
  bool _inFlight = false;
  int _intervalSecs = defaultIntervalSecs;

  ServerReachability status = ServerReachability.unknown;
  DateTime? lastCheckedAt;
  String? lastUrl;
  String? lastError;

  /// Current poll interval in seconds (from lab settings when available).
  int get intervalSecs => _intervalSecs;

  bool get isOnline => status == ServerReachability.online;
  bool get isOffline => status == ServerReachability.offline;

  void _restartTimer() {
    _timer?.cancel();
    _timer = Timer.periodic(Duration(seconds: _intervalSecs), (_) {
      unawaited(check());
    });
  }

  void _applyInterval(int? secs) {
    if (secs == null) return;
    final next = secs.clamp(minIntervalSecs, maxIntervalSecs);
    if (next == _intervalSecs) return;
    _intervalSecs = next;
    _restartTimer();
    notifyListeners();
  }

  Future<void> check() async {
    if (_inFlight) return;
    _inFlight = true;
    if (status != ServerReachability.online) {
      status = ServerReachability.checking;
      notifyListeners();
    }
    // Refresh hardware snapshot so x-device-security-patch / api-level reach the lab.
    await _api.refreshAppVersion();
    final result = await _api.healthCheck();
    lastUrl = result.url;
    lastCheckedAt = DateTime.now();
    if (result.ok) {
      status = ServerReachability.online;
      lastError = null;
      _applyInterval(result.intervalSecs);
    } else {
      status = ServerReachability.offline;
      lastError = result.error;
    }
    _inFlight = false;
    notifyListeners();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      unawaited(check());
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _timer?.cancel();
    super.dispose();
  }
}
