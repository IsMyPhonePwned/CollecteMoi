import 'package:flutter/foundation.dart';

import 'tenant_config.dart';

/// Server URL baked at build time via `--dart-define=COLLECTEMOI_SERVER_URL=…`
/// (see `scripts/collectemoi-app --server`). Empty when not set.
String bundledServerUrlFromDefine() {
  const fromDefine = String.fromEnvironment('COLLECTEMOI_SERVER_URL');
  return fromDefine.trim();
}

/// True when the APK/IPA was generated with an explicit `--server` URL.
bool get hasBundledServerUrl => bundledServerUrlFromDefine().isNotEmpty;

/// Default API base URL for this runtime (from tenant bundle + platform).
///
/// Override order:
/// 1. `--dart-define=COLLECTEMOI_SERVER_URL=…` (see `scripts/collectemoi-app`)
/// 2. persisted AppSettings server URL (set from Settings / enroll)
/// 3. `tenant/default/tenant.json` / platform defaults below
String resolveServerBaseUrl(TenantConfig tenant) {
  final fromDefine = bundledServerUrlFromDefine();
  if (fromDefine.isNotEmpty) return fromDefine;

  if (kIsWeb) return 'https://127.0.0.1:8080';

  // Android emulator reaches the host loopback via 10.0.2.2 (tenant default).
  if (defaultTargetPlatform == TargetPlatform.android) {
    return tenant.serverBaseUrl;
  }

  // iOS simulator / desktop
  return 'https://127.0.0.1:8080';
}
