import 'dart:async';

import 'package:flutter/widgets.dart';

import 'api_client.dart';

enum ServerReachability { unknown, checking, online, offline }

/// Polls `GET /healthz` so the UI can show a live online / offline indicator.
class ServerHealth extends ChangeNotifier with WidgetsBindingObserver {
  ServerHealth(this._api) {
    WidgetsBinding.instance.addObserver(this);
    unawaited(check());
    _timer = Timer.periodic(const Duration(seconds: 20), (_) {
      unawaited(check());
    });
  }

  final ApiClient _api;
  Timer? _timer;
  bool _inFlight = false;

  ServerReachability status = ServerReachability.unknown;
  DateTime? lastCheckedAt;
  String? lastUrl;
  String? lastError;

  bool get isOnline => status == ServerReachability.online;
  bool get isOffline => status == ServerReachability.offline;

  Future<void> check() async {
    if (_inFlight) return;
    _inFlight = true;
    if (status != ServerReachability.online) {
      status = ServerReachability.checking;
      notifyListeners();
    }
    final result = await _api.healthCheck();
    lastUrl = result.url;
    lastCheckedAt = DateTime.now();
    if (result.ok) {
      status = ServerReachability.online;
      lastError = null;
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
