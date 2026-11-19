import 'dart:convert';

import 'package:flutter/services.dart';

class TenantConfig {
  TenantConfig({
    required this.id,
    required this.displayName,
    required this.tagline,
    required this.enrollLead,
    required this.serverBaseUrl,
    required this.tlsPinsSpkiSha256,
    required this.caseEncryptionPublicKeyHex,
    required this.enabledCollectors,
    required this.supportContact,
    required this.privacyPolicyUrl,
    required this.defaultLocale,
    required this.mode,
    required this.copy,
    this.logoAsset,
  });

  final String id;
  final String displayName;
  final String tagline;
  final String enrollLead;
  final String serverBaseUrl;
  final List<String> tlsPinsSpkiSha256;
  final String caseEncryptionPublicKeyHex;
  final List<String> enabledCollectors;
  final String supportContact;
  final String privacyPolicyUrl;
  final String defaultLocale;
  final String mode;
  final TenantCopy copy;
  /// Optional bundled logo path (e.g. tenant/default/assets/logo.png).
  final String? logoAsset;

  bool get hasAndroidBugreport =>
      enabledCollectors.contains('android_bugreport');
  bool get hasIosSysdiagnose => enabledCollectors.contains('ios_sysdiagnose');
  bool get hasMessageReport => enabledCollectors.contains('message_report');
  bool get hasLogo => logoAsset != null && logoAsset!.isNotEmpty;

  factory TenantConfig.fromJson(Map<String, dynamic> json, {String? logoAsset}) {
    return TenantConfig(
      id: json['id'] as String? ?? 'default',
      displayName: json['display_name'] as String? ?? 'CollecteMoi',
      tagline: json['tagline'] as String? ?? '',
      enrollLead: json['enroll_lead'] as String? ??
          'Enter your work email to receive a one-time code, or use a lab key for local testing.',
      serverBaseUrl: json['server_base_url'] as String? ?? '',
      tlsPinsSpkiSha256: (json['tls_pins_spki_sha256'] as List<dynamic>? ?? [])
          .cast<String>(),
      caseEncryptionPublicKeyHex:
          json['case_encryption_public_key_hex'] as String? ?? '',
      enabledCollectors: (json['enabled_collectors'] as List<dynamic>? ??
              ['android_bugreport', 'ios_sysdiagnose', 'message_report'])
          .cast<String>(),
      supportContact: json['support_contact'] as String? ?? '',
      privacyPolicyUrl: json['privacy_policy_url'] as String? ?? '',
      defaultLocale: json['default_locale'] as String? ?? 'en',
      mode: json['mode'] as String? ?? 'generic',
      copy: TenantCopy.fromJson(
        json['copy'] as Map<String, dynamic>? ?? const {},
      ),
      logoAsset: logoAsset,
    );
  }

  TenantConfig copyWith({
    String? serverBaseUrl,
    String? caseEncryptionPublicKeyHex,
    List<String>? enabledCollectors,
    String? displayName,
    String? tagline,
    String? enrollLead,
    String? logoAsset,
    TenantCopy? copy,
  }) {
    return TenantConfig(
      id: id,
      displayName: displayName ?? this.displayName,
      tagline: tagline ?? this.tagline,
      enrollLead: enrollLead ?? this.enrollLead,
      serverBaseUrl: serverBaseUrl ?? this.serverBaseUrl,
      tlsPinsSpkiSha256: tlsPinsSpkiSha256,
      caseEncryptionPublicKeyHex:
          caseEncryptionPublicKeyHex ?? this.caseEncryptionPublicKeyHex,
      enabledCollectors: enabledCollectors ?? this.enabledCollectors,
      supportContact: supportContact,
      privacyPolicyUrl: privacyPolicyUrl,
      defaultLocale: defaultLocale,
      mode: mode,
      copy: copy ?? this.copy,
      logoAsset: logoAsset ?? this.logoAsset,
    );
  }

  static Future<TenantConfig> loadBundled({String id = 'default'}) async {
    final raw = await rootBundle.loadString('tenant/$id/tenant.json');
    final logoPath = 'tenant/$id/assets/logo.png';
    String? logo;
    try {
      await rootBundle.load(logoPath);
      logo = logoPath;
    } catch (_) {
      logo = null;
    }
    return TenantConfig.fromJson(
      jsonDecode(raw) as Map<String, dynamic>,
      logoAsset: logo,
    );
  }
}

class TenantCopy {
  TenantCopy({
    required this.home,
    required this.consent,
    required this.myData,
    required this.androidBugreport,
    required this.iosSysdiagnose,
    required this.messageReport,
  });

  final HomeCopy home;
  final ConsentCopy consent;
  final MyDataCopy myData;
  final GuideCopy androidBugreport;
  final GuideCopy iosSysdiagnose;
  final MessageCopy messageReport;

  factory TenantCopy.fromJson(Map<String, dynamic> json) {
    return TenantCopy(
      home: HomeCopy.fromJson(json['home'] as Map<String, dynamic>? ?? const {}),
      consent: ConsentCopy.fromJson(
        json['consent'] as Map<String, dynamic>? ?? const {},
      ),
      myData: MyDataCopy.fromJson(
        json['my_data'] as Map<String, dynamic>? ?? const {},
      ),
      androidBugreport: GuideCopy.fromJson(
        json['android_bugreport'] as Map<String, dynamic>? ?? const {},
        fallbackTitle: 'Android bugreport',
        fallbackHeadline: 'We need a full system bugreport',
        fallbackBody:
            'Android will create a ZIP with logs, installed apps, and crash data. '
            'You stay in control: you start it from Developer options and share it into this app.',
      ),
      iosSysdiagnose: GuideCopy.fromJson(
        json['ios_sysdiagnose'] as Map<String, dynamic>? ?? const {},
        fallbackTitle: 'iOS sysdiagnose',
        fallbackHeadline: 'Prepare the phone',
        fallbackBody:
            'Unlock the phone, connect to Wi‑Fi, and charge above 30%. '
            'Sysdiagnose is heavy on CPU and storage.',
      ),
      messageReport: MessageCopy.fromJson(
        json['message_report'] as Map<String, dynamic>? ?? const {},
      ),
    );
  }
}

class HomeCopy {
  HomeCopy({
    required this.headline,
    required this.body,
    required this.androidTitle,
    required this.androidSubtitle,
    required this.iosTitle,
    required this.iosSubtitle,
    required this.messageTitle,
    required this.messageSubtitle,
  });

  final String headline;
  final String body;
  final String androidTitle;
  final String androidSubtitle;
  final String iosTitle;
  final String iosSubtitle;
  final String messageTitle;
  final String messageSubtitle;

  factory HomeCopy.fromJson(Map<String, dynamic> json) {
    return HomeCopy(
      headline: json['headline'] as String? ?? 'What do you need to send?',
      body: json['body'] as String? ??
          'Your analyst asked for evidence from this phone. '
              'Follow the guided steps — we never collect anything without your consent.',
      androidTitle: json['android_title'] as String? ?? 'Android bugreport',
      androidSubtitle: json['android_subtitle'] as String? ??
          'System diagnostic ZIP via Developer options',
      iosTitle: json['ios_title'] as String? ?? 'iOS sysdiagnose',
      iosSubtitle: json['ios_subtitle'] as String? ??
          'Hardware button combo, then share the archive',
      messageTitle: json['message_title'] as String? ?? 'Suspicious message',
      messageSubtitle: json['message_subtitle'] as String? ??
          'Share, screenshot, or paste the message',
    );
  }
}

class ConsentCopy {
  ConsentCopy({
    required this.title,
    required this.footer,
    required this.checkbox,
    required this.button,
    required this.disclosureAndroid,
    required this.disclosureIos,
    required this.disclosureMessage,
  });

  final String title;
  final String footer;
  final String checkbox;
  final String button;
  final String disclosureAndroid;
  final String disclosureIos;
  final String disclosureMessage;

  factory ConsentCopy.fromJson(Map<String, dynamic> json) {
    return ConsentCopy(
      title: json['title'] as String? ?? 'Consent before upload',
      footer: json['footer'] as String? ??
          'The file is encrypted on this phone to the case key before upload. '
              'You can review or discard later from My data.',
      checkbox: json['checkbox'] as String? ??
          'I understand and consent to send this artefact',
      button: json['button'] as String? ?? 'Encrypt and upload',
      disclosureAndroid: json['disclosure_android'] as String? ??
          'A bugreport can include installed apps, system logs, Wi‑Fi networks, '
              'account identifiers, and sometimes location history.',
      disclosureIos: json['disclosure_ios'] as String? ??
          'A sysdiagnose can include process lists, system logs, crash reports, '
              'network details, and other diagnostics collected by iOS.',
      disclosureMessage: json['disclosure_message'] as String? ??
          'The message text, metadata you entered, and any screenshots will be '
              'sent encrypted to the analyst handling your case.',
    );
  }
}

class MyDataCopy {
  MyDataCopy({
    required this.title,
    required this.deviceHeading,
    required this.artefactsHeading,
    required this.empty,
    required this.clearButton,
  });

  final String title;
  final String deviceHeading;
  final String artefactsHeading;
  final String empty;
  final String clearButton;

  factory MyDataCopy.fromJson(Map<String, dynamic> json) {
    return MyDataCopy(
      title: json['title'] as String? ?? 'My data',
      deviceHeading: json['device_heading'] as String? ?? 'Device',
      artefactsHeading:
          json['artefacts_heading'] as String? ?? 'Artefacts on this phone',
      empty: json['empty'] as String? ?? 'Nothing queued yet.',
      clearButton: json['clear_button'] as String? ?? 'Clear local enrollment',
    );
  }
}

class GuideCopy {
  GuideCopy({
    required this.title,
    required this.headline,
    required this.body,
  });

  final String title;
  final String headline;
  final String body;

  factory GuideCopy.fromJson(
    Map<String, dynamic> json, {
    required String fallbackTitle,
    required String fallbackHeadline,
    required String fallbackBody,
  }) {
    return GuideCopy(
      title: json['title'] as String? ?? fallbackTitle,
      headline: json['headline'] as String? ?? fallbackHeadline,
      body: json['body'] as String? ?? fallbackBody,
    );
  }
}

class MessageCopy {
  MessageCopy({
    required this.title,
    required this.lead,
    required this.button,
  });

  final String title;
  final String lead;
  final String button;

  factory MessageCopy.fromJson(Map<String, dynamic> json) {
    return MessageCopy(
      title: json['title'] as String? ?? 'Suspicious message',
      lead: json['lead'] as String? ??
          'We never open links on this device. Confirm any URL we detect below.',
      button: json['button'] as String? ?? 'Continue to consent',
    );
  }
}

class ThemeConfig {
  ThemeConfig({
    required this.seedColor,
    required this.light,
    required this.dark,
  });

  final String seedColor;
  final Map<String, String> light;
  final Map<String, String> dark;

  factory ThemeConfig.fromJson(Map<String, dynamic> json) {
    return ThemeConfig(
      seedColor: json['seed_color'] as String,
      light: (json['light'] as Map<String, dynamic>).map(
        (k, v) => MapEntry(k, v as String),
      ),
      dark: (json['dark'] as Map<String, dynamic>).map(
        (k, v) => MapEntry(k, v as String),
      ),
    );
  }

  static Future<ThemeConfig> loadBundled({String id = 'default'}) async {
    final raw = await rootBundle.loadString('tenant/$id/theme.json');
    return ThemeConfig.fromJson(jsonDecode(raw) as Map<String, dynamic>);
  }
}
