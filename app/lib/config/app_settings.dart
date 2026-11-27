import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'lab_http_overrides_stub.dart'
    if (dart.library.io) 'lab_http_overrides_io.dart';

/// Device-local preferences that are not part of tenant branding.
class AppSettings extends ChangeNotifier {
  AppSettings(
    this._preferences, {
    required this.showLabKey,
    required this.trustSelfSigned,
    required this.serverBaseUrl,
  });

  static const _labKeyStorage = 'collectemoi-app-show-lab-key';
  static const _trustSelfSignedStorage = 'collectemoi-app-trust-self-signed';
  static const _serverUrlStorage = 'collectemoi-app-server-url';

  final SharedPreferences _preferences;

  /// When false (default), enroll hides the lab-key field and QR scanner.
  bool showLabKey;

  /// When true, accept the lab server's self-signed TLS certificate
  /// (non-loopback hosts). Loopback is always allowed.
  bool trustSelfSigned;

  /// API base URL used for enrollment (and before a session exists).
  /// Prefers a user override from Settings; otherwise the build/tenant default.
  String serverBaseUrl;

  static Future<AppSettings> load({required String defaultServerUrl}) async {
    final preferences = await SharedPreferences.getInstance();
    final trustSelfSigned =
        preferences.getBool(_trustSelfSignedStorage) ?? false;
    applyLabTlsPreference(trustSelfSigned: trustSelfSigned);
    final saved = preferences.getString(_serverUrlStorage)?.trim() ?? '';
    final serverBaseUrl = saved.isNotEmpty
        ? saved.replaceAll(RegExp(r'/$'), '')
        : defaultServerUrl.replaceAll(RegExp(r'/$'), '');
    return AppSettings(
      preferences,
      showLabKey: preferences.getBool(_labKeyStorage) ?? false,
      trustSelfSigned: trustSelfSigned,
      serverBaseUrl: serverBaseUrl,
    );
  }

  Future<void> setShowLabKey(bool value) async {
    if (value == showLabKey) return;
    showLabKey = value;
    notifyListeners();
    await _preferences.setBool(_labKeyStorage, value);
  }

  Future<void> setTrustSelfSigned(bool value) async {
    if (value == trustSelfSigned) return;
    trustSelfSigned = value;
    applyLabTlsPreference(trustSelfSigned: value);
    notifyListeners();
    await _preferences.setBool(_trustSelfSignedStorage, value);
  }

  Future<void> setServerBaseUrl(String value) async {
    final normalized = value.trim().replaceAll(RegExp(r'/$'), '');
    if (normalized.isEmpty || normalized == serverBaseUrl) return;
    serverBaseUrl = normalized;
    notifyListeners();
    await _preferences.setString(_serverUrlStorage, normalized);
  }
}
