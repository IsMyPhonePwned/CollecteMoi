import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/foundation.dart';
import 'package:package_info_plus/package_info_plus.dart';

/// Snapshot of the phone/browser collected at enroll time (and shown in My data).
class DeviceInfoSnapshot {
  const DeviceInfoSnapshot({
    required this.platform,
    required this.model,
    required this.osVersion,
    this.manufacturer = '',
    this.brand = '',
    this.name = '',
    this.details = const {},
  });

  /// API `platform`: android | ios | web | other
  final String platform;

  /// Human-readable device model for API + UI (e.g. "Pixel 8", "iPhone 15").
  final String model;

  /// Exact OS line for admin (e.g. "Android 15 (API 35) · patch 2025-03-05").
  final String osVersion;

  final String manufacturer;
  final String brand;
  final String name;
  final Map<String, String> details;

  String get securityPatch => details['security_patch'] ?? '';
  String get apiLevel => details['api_level'] ?? details['sdk'] ?? '';

  String get summary {
    final parts = <String>[
      if (manufacturer.isNotEmpty &&
          !model.toLowerCase().startsWith(manufacturer.toLowerCase()))
        manufacturer,
      if (model.isNotEmpty) model,
      if (osVersion.isNotEmpty) osVersion,
    ];
    return parts.isEmpty ? platform : parts.join(' · ');
  }

  Map<String, dynamic> toJson() => {
    'platform': platform,
    'model': model,
    'os_version': osVersion,
    if (manufacturer.isNotEmpty) 'manufacturer': manufacturer,
    if (brand.isNotEmpty) 'brand': brand,
    if (name.isNotEmpty) 'name': name,
    if (details.isNotEmpty) 'details': details,
  };

  factory DeviceInfoSnapshot.fromJson(Map<String, dynamic>? json) {
    if (json == null) {
      return const DeviceInfoSnapshot(
        platform: 'unknown',
        model: '',
        osVersion: '',
      );
    }
    final rawDetails = json['details'];
    final details = <String, String>{};
    if (rawDetails is Map) {
      for (final e in rawDetails.entries) {
        final v = e.value?.toString().trim() ?? '';
        if (v.isNotEmpty) details[e.key.toString()] = v;
      }
    }
    return DeviceInfoSnapshot(
      platform: (json['platform'] as String?)?.trim().isNotEmpty == true
          ? (json['platform'] as String).trim()
          : 'unknown',
      model: (json['model'] as String?)?.trim() ?? '',
      osVersion:
          (json['os_version'] as String?)?.trim() ??
          (json['osVersion'] as String?)?.trim() ??
          '',
      manufacturer: (json['manufacturer'] as String?)?.trim() ?? '',
      brand: (json['brand'] as String?)?.trim() ?? '',
      name: (json['name'] as String?)?.trim() ?? '',
      details: details,
    );
  }
}

class DeviceInfoService {
  DeviceInfoService._();

  static final _plugin = DeviceInfoPlugin();

  /// `version+buildNumber` for API headers (e.g. `0.1.0+1`).
  static Future<String> appVersion() async {
    try {
      final info = await PackageInfo.fromPlatform();
      return '${info.version}+${info.buildNumber}';
    } catch (_) {
      return '';
    }
  }

  static Future<DeviceInfoSnapshot> capture() async {
    if (kIsWeb) {
      final info = await _plugin.webBrowserInfo;
      final browser = info.browserName.name;
      final ua = info.userAgent?.trim() ?? '';
      return DeviceInfoSnapshot(
        platform: 'web',
        model: browser.isEmpty ? 'browser' : browser,
        osVersion: info.platform?.trim().isNotEmpty == true
            ? info.platform!.trim()
            : 'web',
        name: info.appName?.trim() ?? '',
        details: {
          if (ua.isNotEmpty) 'user_agent': ua,
          if ((info.vendor ?? '').trim().isNotEmpty) 'vendor': info.vendor!.trim(),
          if ((info.language ?? '').trim().isNotEmpty)
            'language': info.language!.trim(),
        },
      );
    }

    switch (defaultTargetPlatform) {
      case TargetPlatform.android:
        final a = await _plugin.androidInfo;
        final manufacturer = a.manufacturer.trim();
        final model = a.model.trim().isNotEmpty ? a.model.trim() : a.device;
        final brand = a.brand.trim();
        final release = a.version.release.trim();
        final sdk = a.version.sdkInt;
        final patch = (a.version.securityPatch ?? '').trim();
        final incremental = a.version.incremental.trim();
        final codename = a.version.codename.trim();
        final baseOs = (a.version.baseOS ?? '').trim();
        final previewSdk = a.version.previewSdkInt;

        final osBits = <String>[
          if (release.isNotEmpty) 'Android $release' else 'Android',
          '(API $sdk)',
          if (patch.isNotEmpty) '· patch $patch',
        ];

        return DeviceInfoSnapshot(
          platform: 'android',
          model: model,
          osVersion: osBits.join(' '),
          manufacturer: manufacturer,
          brand: brand,
          name: a.device.trim().isNotEmpty ? a.device.trim() : a.name.trim(),
          details: {
            'api_level': '$sdk',
            'sdk': '$sdk',
            if (release.isNotEmpty) 'android_release': release,
            if (patch.isNotEmpty) 'security_patch': patch,
            if (incremental.isNotEmpty) 'build_incremental': incremental,
            if (codename.isNotEmpty && codename != 'REL') 'codename': codename,
            if (baseOs.isNotEmpty) 'base_os': baseOs,
            if (previewSdk != null && previewSdk > 0)
              'preview_sdk': '$previewSdk',
            if (a.id.trim().isNotEmpty) 'build_id': a.id.trim(),
            if (a.display.trim().isNotEmpty) 'build_display': a.display.trim(),
            if (a.type.trim().isNotEmpty) 'build_type': a.type.trim(),
            if (a.tags.trim().isNotEmpty) 'build_tags': a.tags.trim(),
            if (a.board.trim().isNotEmpty) 'board': a.board.trim(),
            if (a.bootloader.trim().isNotEmpty) 'bootloader': a.bootloader.trim(),
            if (a.hardware.trim().isNotEmpty) 'hardware': a.hardware.trim(),
            if (a.product.trim().isNotEmpty) 'product': a.product.trim(),
            if (a.host.trim().isNotEmpty) 'build_host': a.host.trim(),
            if (a.supportedAbis.isNotEmpty) 'abis': a.supportedAbis.join(', '),
            if (a.isPhysicalDevice) 'physical': 'yes' else 'physical': 'emulator',
            if (a.fingerprint.trim().isNotEmpty)
              'fingerprint': a.fingerprint.trim(),
          },
        );
      case TargetPlatform.iOS:
        final i = await _plugin.iosInfo;
        final name = i.name.trim();
        final machine = i.utsname.machine.trim();
        final modelName = i.modelName.trim();
        final model = modelName.isNotEmpty
            ? modelName
            : (machine.isNotEmpty ? machine : i.model.trim());
        final version = i.systemVersion.trim();
        final systemName = i.systemName.trim();
        final kernel = i.utsname.release.trim();
        final kernelVersion = i.utsname.version.trim();

        final osLabel = [
          if (systemName.isNotEmpty) systemName else 'iOS',
          if (version.isNotEmpty) version,
        ].join(' ');

        return DeviceInfoSnapshot(
          platform: 'ios',
          model: model.isNotEmpty ? model : 'iPhone',
          osVersion: osLabel,
          manufacturer: 'Apple',
          brand: 'Apple',
          name: name,
          details: {
            if (systemName.isNotEmpty) 'system': systemName,
            if (version.isNotEmpty) 'ios_version': version,
            if (i.model.trim().isNotEmpty) 'model_id': i.model.trim(),
            if (modelName.isNotEmpty) 'model_name': modelName,
            if (i.localizedModel.trim().isNotEmpty)
              'localized_model': i.localizedModel.trim(),
            if (machine.isNotEmpty) 'machine': machine,
            if (kernel.isNotEmpty) 'darwin_release': kernel,
            if (kernelVersion.isNotEmpty) 'darwin_version': kernelVersion,
            if (i.isPhysicalDevice) 'physical': 'yes' else 'physical': 'simulator',
            if (i.identifierForVendor != null)
              'idfv': i.identifierForVendor!,
          },
        );
      default:
        return DeviceInfoSnapshot(
          platform: 'other',
          model: defaultTargetPlatform.name,
          osVersion: defaultTargetPlatform.name,
        );
    }
  }
}
