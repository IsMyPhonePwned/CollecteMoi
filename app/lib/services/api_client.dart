import 'dart:convert';
import 'dart:typed_data';

import 'package:http/http.dart' as http;
import 'package:package_info_plus/package_info_plus.dart';

import '../crypto/noise_wrap.dart';
import '../models/models.dart';
import 'device_info_service.dart' show DeviceInfoService, DeviceInfoSnapshot;

/// Identity attached to every API packet (headers + enroll body).
class ClientIdentity {
  const ClientIdentity({
    this.deviceId,
    this.appVersion = '',
    this.platform = '',
    this.model = '',
    this.osVersion = '',
    this.manufacturer = '',
    this.brand = '',
    this.name = '',
    this.details = const {},
  });

  final String? deviceId;
  final String appVersion;
  final String platform;
  final String model;
  final String osVersion;
  final String manufacturer;
  final String brand;
  final String name;
  final Map<String, String> details;

  String get securityPatch =>
      details['security_patch'] ?? details['securityPatch'] ?? '';
  String get apiLevel => details['api_level'] ?? details['sdk'] ?? '';
  String get buildId => details['build_id'] ?? '';
  String get buildDisplay =>
      details['build_display'] ?? details['android_build'] ?? '';
  String get buildIncremental => details['build_incremental'] ?? '';
  String get buildType => details['build_type'] ?? '';
  String get androidRelease => details['android_release'] ?? '';
  String get physicalState => details['physical'] ?? '';
  String get lowRamState => details['low_ram'] ?? '';

  ClientIdentity copyWith({
    String? deviceId,
    String? appVersion,
    String? platform,
    String? model,
    String? osVersion,
    String? manufacturer,
    String? brand,
    String? name,
    Map<String, String>? details,
    bool clearDeviceId = false,
  }) {
    return ClientIdentity(
      deviceId: clearDeviceId ? null : (deviceId ?? this.deviceId),
      appVersion: appVersion ?? this.appVersion,
      platform: platform ?? this.platform,
      model: model ?? this.model,
      osVersion: osVersion ?? this.osVersion,
      manufacturer: manufacturer ?? this.manufacturer,
      brand: brand ?? this.brand,
      name: name ?? this.name,
      details: details ?? this.details,
    );
  }

  factory ClientIdentity.fromSession(
    EnrollmentSession session, {
    String? appVersion,
  }) {
    return ClientIdentity(
      deviceId: session.deviceId,
      appVersion: appVersion ?? session.appVersion ?? '',
      platform: session.devicePlatform ?? '',
      model: session.deviceModel ?? '',
      osVersion: session.deviceOsVersion ?? '',
      manufacturer: session.deviceManufacturer ?? '',
      brand: session.deviceBrand ?? '',
      name: session.deviceName ?? '',
      details: session.deviceDetails,
    );
  }

  factory ClientIdentity.fromDevice(
    DeviceInfoSnapshot device, {
    required String appVersion,
    String? deviceId,
  }) {
    return ClientIdentity(
      deviceId: deviceId,
      appVersion: appVersion,
      platform: device.platform,
      model: device.model,
      osVersion: device.osVersion,
      manufacturer: device.manufacturer,
      brand: device.brand,
      name: device.name,
      details: device.details,
    );
  }

  Map<String, dynamic> toEnrollBodyFields() => {
        'platform': platform,
        'model': model.isEmpty ? platform : model,
        'os_version': osVersion.isEmpty ? platform : osVersion,
        if (appVersion.isNotEmpty) 'app_version': appVersion,
        if (manufacturer.isNotEmpty) 'manufacturer': manufacturer,
        if (brand.isNotEmpty) 'brand': brand,
        if (name.isNotEmpty) 'name': name,
        if (details.isNotEmpty) 'details': details,
      };
}

class ApiClient {
  ApiClient({required this.baseUrl, http.Client? client})
      : _client = client ?? http.Client();

  String baseUrl;
  final http.Client _client;
  ClientIdentity identity = const ClientIdentity();

  Uri _u(String path) => Uri.parse('$baseUrl$path');

  /// Apply enrolled session identity (device id + hardware snapshot + version).
  void applySession(EnrollmentSession? session) {
    if (session == null) {
      identity = identity.copyWith(clearDeviceId: true);
      return;
    }
    identity = ClientIdentity.fromSession(
      session,
      appVersion: session.appVersion ?? identity.appVersion,
    );
  }

  /// Refresh PackageInfo + live device snapshot into identity (boot / enroll).
  Future<void> refreshAppVersion() async {
    try {
      final info = await PackageInfo.fromPlatform();
      final version = '${info.version}+${info.buildNumber}';
      identity = identity.copyWith(appVersion: version);
    } catch (_) {
      // Keep previous version if PackageInfo is unavailable (tests).
    }
    try {
      final device = await DeviceInfoService.capture();
      identity = identity.copyWith(
        platform: device.platform,
        model: device.model,
        osVersion: device.osVersion,
        manufacturer: device.manufacturer,
        brand: device.brand,
        name: device.name,
        details: device.details,
      );
    } catch (_) {
      // Keep previous device fields if capture fails.
    }
  }

  /// HTTP header values must be printable ASCII (Dart HttpClient rejects
  /// Unicode / CR/LF). Device names and models often include non-ASCII.
  static String sanitizeHeaderValue(String value, {int maxLen = 120}) {
    final out = StringBuffer();
    for (final unit in value.codeUnits) {
      // Allow TAB and visible ASCII only (RFC 7230 field-vchar / obs-text avoided).
      if (unit == 0x09 || (unit >= 0x20 && unit <= 0x7e)) {
        out.writeCharCode(unit);
      }
    }
    var s = out.toString().trim();
    if (s.length > maxLen) s = s.substring(0, maxLen).trim();
    return s;
  }

  Map<String, String> _headers([Map<String, String>? extra]) {
    final id = identity;
    String h(String value, {int maxLen = 120}) =>
        sanitizeHeaderValue(value, maxLen: maxLen);

    final uaPlatform = h(id.platform.isEmpty ? 'unknown' : id.platform, maxLen: 32);
    final uaModel = h(id.model.isEmpty ? uaPlatform : id.model, maxLen: 64);
    final uaVersion = h(id.appVersion.isEmpty ? 'dev' : id.appVersion, maxLen: 32);
    final headers = <String, String>{
      'user-agent': 'CollecteMoi/$uaVersion ($uaPlatform; $uaModel)',
    };
    void put(String name, String value, {int maxLen = 120}) {
      final v = h(value, maxLen: maxLen);
      if (v.isNotEmpty) headers[name] = v;
    }

    put('x-device-id', id.deviceId ?? '', maxLen: 80);
    put('x-app-version', id.appVersion, maxLen: 64);
    put('x-device-platform', id.platform, maxLen: 32);
    put('x-device-model', id.model, maxLen: 80);
    put('x-device-os', id.osVersion, maxLen: 80);
    put('x-device-manufacturer', id.manufacturer, maxLen: 64);
    put('x-device-brand', id.brand, maxLen: 64);
    put('x-device-name', id.name, maxLen: 120);
    put('x-device-security-patch', id.securityPatch, maxLen: 32);
    put('x-device-api-level', id.apiLevel, maxLen: 16);
    put('x-device-build-id', id.buildId, maxLen: 64);
    put('x-device-build-display', id.buildDisplay, maxLen: 96);
    put('x-device-build-incremental', id.buildIncremental, maxLen: 64);
    put('x-device-build-type', id.buildType, maxLen: 32);
    put('x-device-android-release', id.androidRelease, maxLen: 32);
    put('x-device-physical', id.physicalState, maxLen: 16);
    put('x-device-low-ram', id.lowRamState, maxLen: 8);
    put('x-device-ram-mb', id.details['ram_mb'] ?? '', maxLen: 16);
    put('x-device-disk-free-mb', id.details['disk_free_mb'] ?? '', maxLen: 16);

    if (extra != null) {
      for (final e in extra.entries) {
        final v = h(e.value, maxLen: 256);
        if (v.isNotEmpty) headers[e.key] = v;
      }
    }
    return headers;
  }

  Future<Map<String, dynamic>> enrollStart({
    String? email,
    String? labKey,
  }) async {
    final res = await _client.post(
      _u('/v1/enroll/start'),
      headers: _headers({'content-type': 'application/json'}),
      body: jsonEncode({
        if (email != null && email.isNotEmpty) 'email': email,
        if (labKey != null && labKey.isNotEmpty) 'lab_key': labKey,
      }),
    );
    _ensureOk(res);
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  Future<Map<String, dynamic>> enrollConfirm({
    required String challengeId,
    required String code,
    required String devicePublicKeyHex,
    required String noiseKeyHex,
    required ClientIdentity device,
  }) async {
    identity = device.copyWith(deviceId: identity.deviceId);
    final res = await _client.post(
      _u('/v1/enroll/confirm'),
      headers: _headers({'content-type': 'application/json'}),
      body: jsonEncode({
        'challenge_id': challengeId,
        'code': code,
        'device_public_key': devicePublicKeyHex,
        'noise_key': noiseKeyHex,
        ...device.toEnrollBodyFields(),
      }),
    );
    _ensureOk(res);
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  Future<Map<String, dynamic>> enroll({
    required String token,
    required String devicePublicKeyHex,
    required String noiseKeyHex,
    required ClientIdentity device,
  }) async {
    identity = device.copyWith(deviceId: identity.deviceId);
    final res = await _client.post(
      _u('/v1/enroll'),
      headers: _headers({'content-type': 'application/json'}),
      body: jsonEncode({
        'token': token,
        'device_public_key': devicePublicKeyHex,
        'noise_key': noiseKeyHex,
        ...device.toEnrollBodyFields(),
      }),
    );
    _ensureOk(res);
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  Future<Map<String, dynamic>> tenantManifest() async {
    final res = await _client.get(
      _u('/v1/tenant/manifest'),
      headers: _headers(),
    );
    _ensureOk(res);
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  /// Public lab status (includes server `display_name`).
  Future<Map<String, dynamic>> publicStatus() async {
    final res = await _client.get(
      _u('/v1/status'),
      headers: _headers(),
    );
    _ensureOk(res);
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  /// `GET /healthz` — reachability plus a short error detail when offline.
  ///
  /// When enrolled, sends `x-device-*` headers so the lab can attribute the
  /// check-in to this device (Devices → Last seen). Successful responses may
  /// include `x-collectemoi-healthz-interval` (seconds) from lab Settings.
  Future<HealthCheckResult> healthCheck({
    Duration timeout = const Duration(seconds: 4),
  }) async {
    final url = _u('/healthz').toString();
    try {
      final headers = _headers();
      final res =
          await _client.get(_u('/healthz'), headers: headers).timeout(timeout);
      if (res.statusCode != 200) {
        final body = res.body.trim();
        return HealthCheckResult.fail(
          url: url,
          error:
              'HTTP ${res.statusCode}${body.isEmpty ? '' : ': ${body.length > 180 ? '${body.substring(0, 180)}…' : body}'}',
        );
      }
      final intervalSecs = _parseHealthzInterval(res.headers);
      final body = res.body.trim().toLowerCase();
      if (body.isEmpty || body == 'ok') {
        return HealthCheckResult.ok(url: url, intervalSecs: intervalSecs);
      }
      return HealthCheckResult.fail(
        url: url,
        error: 'Unexpected health response: ${res.body.trim()}',
      );
    } on ArgumentError catch (e) {
      // Typical when a device field still slips past sanitization.
      return HealthCheckResult.fail(
        url: url,
        error: 'Invalid request header: ${e.message}',
      );
    } catch (e) {
      return HealthCheckResult.fail(url: url, error: e.toString());
    }
  }

  static int? _parseHealthzInterval(Map<String, String> headers) {
    final raw = headers['x-collectemoi-healthz-interval'] ??
        headers['X-Collectemoi-Healthz-Interval'];
    if (raw == null || raw.trim().isEmpty) return null;
    return int.tryParse(raw.trim());
  }

  Future<List<CollectionRequest>> requests() async {
    final res = await _client.get(
      _u('/v1/requests'),
      headers: _headers(),
    );
    _ensureOk(res);
    final list = jsonDecode(res.body) as List<dynamic>;
    return list
        .map((e) => CollectionRequest.fromJson(e as Map<String, dynamic>))
        .toList();
  }

  Future<String> createUpload({
    required String caseId,
    required ArtefactKind artefact,
    required int totalSize,
    required int chunkSize,
    required int chunkCount,
    required String ciphertextSha256,
  }) async {
    final res = await _client.post(
      _u('/v1/uploads'),
      headers: _headers({'content-type': 'application/json'}),
      body: jsonEncode({
        'case_id': caseId,
        'artefact': artefact.apiValue,
        'total_size': totalSize,
        'chunk_size': chunkSize,
        'chunk_count': chunkCount,
        'ciphertext_sha256': ciphertextSha256,
      }),
    );
    _ensureOk(res);
    return (jsonDecode(res.body) as Map<String, dynamic>)['upload_id']
        as String;
  }

  Future<void> putChunk({
    required String uploadId,
    required int index,
    required List<int> bytes,
    required String sha256Hex,
  }) async {
    final res = await _client.put(
      _u('/v1/uploads/$uploadId/chunks/$index'),
      headers: _headers({
        'content-type': 'application/octet-stream',
        'digest': 'sha-256=$sha256Hex',
      }),
      body: bytes,
    );
    if (res.statusCode != 204 && res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
  }

  Future<void> completeUpload({
    required String uploadId,
    required String devicePublicKeyHex,
  }) async {
    final res = await _client.post(
      _u('/v1/uploads/$uploadId/complete'),
      headers: _headers({'content-type': 'application/json'}),
      body: jsonEncode({'device_public_key': devicePublicKeyHex}),
    );
    _ensureOk(res);
  }

  Future<List<ServerSubmission>> clientSubmissions({
    required String deviceId,
    required String noiseKeyHex,
  }) async {
    final proof = await NoiseWrap.seal(
      plaintext: Uint8List.fromList(
        utf8.encode('collectemoi-client-submissions-v1:$deviceId'),
      ),
      noiseKeyHex: noiseKeyHex,
    );
    identity = identity.copyWith(deviceId: deviceId);
    final res = await _client.post(
      _u('/v1/client/submissions'),
      headers: _headers({'content-type': 'application/octet-stream'}),
      body: proof,
    );
    _ensureOk(res);
    final items = jsonDecode(res.body) as List<dynamic>;
    return items
        .map((item) => ServerSubmission.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  Future<void> submitMessage(Map<String, dynamic> report) async {
    final res = await _client.post(
      _u('/v1/messages'),
      headers: _headers({'content-type': 'application/json'}),
      body: jsonEncode(report),
    );
    _ensureOk(res);
  }

  void _ensureOk(http.Response res) {
    if (res.statusCode < 200 || res.statusCode >= 300) {
      throw ApiException(res.statusCode, res.body);
    }
  }
}

class ApiException implements Exception {
  ApiException(this.statusCode, this.body);
  final int statusCode;
  final String body;

  /// Server error code when JSON `{"error":"…"}` is present.
  String get errorCode {
    try {
      final decoded = jsonDecode(body);
      if (decoded is Map && decoded['error'] is String) {
        return (decoded['error'] as String).trim();
      }
    } catch (_) {}
    return body.trim();
  }

  bool get isDeviceBanned =>
      statusCode == 403 &&
      (errorCode == 'device_banned' ||
          errorCode.toLowerCase().contains('banned'));

  /// Server per-device upload quota (`POST /v1/uploads`).
  bool get isDeviceRateLimited =>
      statusCode == 429 &&
      (errorCode == 'device_rate_limited' ||
          errorCode.toLowerCase().contains('device_rate_limited'));

  bool get isTransient =>
      statusCode == 408 ||
      statusCode == 429 ||
      statusCode >= 500 ||
      statusCode == 0;

  @override
  String toString() => 'ApiException($statusCode): $body';
}

class HealthCheckResult {
  const HealthCheckResult._({
    required this.ok,
    required this.url,
    this.error,
    this.intervalSecs,
  });

  factory HealthCheckResult.ok({
    required String url,
    int? intervalSecs,
  }) =>
      HealthCheckResult._(ok: true, url: url, intervalSecs: intervalSecs);

  factory HealthCheckResult.fail({
    required String url,
    required String error,
  }) => HealthCheckResult._(ok: false, url: url, error: error);

  final bool ok;
  final String url;
  final String? error;

  /// Lab-configured poll interval from `x-collectemoi-healthz-interval`.
  final int? intervalSecs;
}
