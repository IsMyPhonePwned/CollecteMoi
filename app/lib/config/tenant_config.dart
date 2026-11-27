import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/app_localizations.dart';

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
    required this.enabledLocales,
    required this.mode,
    required this.copy,
    required this.copyI18n,
    required this.stringsI18n,
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
  final List<String> enabledLocales;
  final String mode;
  final TenantCopy copy;
  /// Per-locale copy overrides (`fr` / `es`). Empty fields fall back to dictionary.
  final Map<String, TenantCopy> copyI18n;
  /// Per-locale overrides for tagline / enroll / OTP strings.
  final Map<String, TenantStringsI18n> stringsI18n;
  /// Optional bundled logo path (e.g. tenant/default/assets/logo.png).
  final String? logoAsset;

  List<Locale> get supportedLocales {
    final codes = enabledLocales.isEmpty
        ? const ['en', 'fr', 'es']
        : enabledLocales;
    final out = <Locale>[];
    for (final code in codes) {
      final c = code.toLowerCase().split(RegExp('[-_]')).first;
      if (c.isEmpty) continue;
      if (!out.any((l) => l.languageCode == c)) {
        out.add(Locale(c));
      }
    }
    if (out.isEmpty) return appSupportedLocales;
    if (!out.any((l) => l.languageCode == 'en')) {
      out.insert(0, const Locale('en'));
    }
    return out;
  }

  bool get hasAndroidBugreport =>
      enabledCollectors.contains('android_bugreport');
  bool get hasIosSysdiagnose => enabledCollectors.contains('ios_sysdiagnose');
  bool get hasMessageReport => enabledCollectors.contains('message_report');
  bool get hasLogo => logoAsset != null && logoAsset!.isNotEmpty;

  factory TenantConfig.fromJson(Map<String, dynamic> json, {String? logoAsset}) {
    final copyI18nRaw = json['copy_i18n'] as Map<String, dynamic>? ?? const {};
    final stringsI18nRaw =
        json['strings_i18n'] as Map<String, dynamic>? ?? const {};
    final enabled = (json['enabled_locales'] as List<dynamic>? ??
            ['en', 'fr', 'es'])
        .map((e) => e.toString())
        .where((e) => e.trim().isNotEmpty)
        .toList();
    return TenantConfig(
      id: json['id'] as String? ?? 'default',
      displayName: json['display_name'] as String? ?? 'CollecteMoi',
      tagline: json['tagline'] as String? ?? '',
      enrollLead: json['enroll_lead'] as String? ??
          'Enter your work email to receive a one-time verification code.',
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
      enabledLocales: enabled.isEmpty ? ['en', 'fr', 'es'] : enabled,
      mode: json['mode'] as String? ?? 'generic',
      copy: TenantCopy.fromJson(
        json['copy'] as Map<String, dynamic>? ?? const {},
      ),
      copyI18n: copyI18nRaw.map(
        (k, v) => MapEntry(
          k,
          TenantCopy.fromJson(
            v is Map<String, dynamic> ? v : const <String, dynamic>{},
            loose: true,
          ),
        ),
      ),
      stringsI18n: stringsI18nRaw.map(
        (k, v) => MapEntry(
          k,
          TenantStringsI18n.fromJson(
            v is Map<String, dynamic> ? v : const <String, dynamic>{},
          ),
        ),
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
    List<String>? enabledLocales,
    Map<String, TenantCopy>? copyI18n,
    Map<String, TenantStringsI18n>? stringsI18n,
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
      enabledLocales: enabledLocales ?? this.enabledLocales,
      mode: mode,
      copy: copy ?? this.copy,
      copyI18n: copyI18n ?? this.copyI18n,
      stringsI18n: stringsI18n ?? this.stringsI18n,
      logoAsset: logoAsset ?? this.logoAsset,
    );
  }

  /// Resolve a branded string for [locale]: explicit override → dictionary → English.
  String branded(
    Locale locale,
    String Function(TenantCopy copy) pick, {
    String Function(TenantStringsI18n strings)? pickStrings,
    String? englishFallback,
  }) {
    final lang = locale.languageCode.toLowerCase();
    if (pickStrings != null) {
      final over = stringsI18n[lang];
      if (over != null) {
        final v = pickStrings(over).trim();
        if (v.isNotEmpty) return v;
      }
      final english = englishFallback ?? '';
      return AppLocalizations.translate(english, locale);
    }
    final over = copyI18n[lang];
    if (over != null) {
      final v = pick(over).trim();
      if (v.isNotEmpty) return v;
    }
    return AppLocalizations.translate(pick(copy), locale);
  }

  String brandedTagline(Locale locale) => branded(
        locale,
        (_) => '',
        pickStrings: (s) => s.tagline,
        englishFallback: tagline,
      );

  String brandedEnrollLead(Locale locale) => branded(
        locale,
        (_) => '',
        pickStrings: (s) => s.enrollLead,
        englishFallback: enrollLead,
      );

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
    required this.notifications,
  });

  final HomeCopy home;
  final ConsentCopy consent;
  final MyDataCopy myData;
  final GuideCopy androidBugreport;
  final GuideCopy iosSysdiagnose;
  final MessageCopy messageReport;
  final NotificationsCopy notifications;

  factory TenantCopy.fromJson(
    Map<String, dynamic> json, {
    bool loose = false,
  }) {
    final empty = loose ? '' : null;
    return TenantCopy(
      home: HomeCopy.fromJson(
        json['home'] as Map<String, dynamic>? ?? const {},
        loose: loose,
      ),
      consent: ConsentCopy.fromJson(
        json['consent'] as Map<String, dynamic>? ?? const {},
        loose: loose,
      ),
      myData: MyDataCopy.fromJson(
        json['my_data'] as Map<String, dynamic>? ?? const {},
        loose: loose,
      ),
      androidBugreport: GuideCopy.fromJson(
        json['android_bugreport'] as Map<String, dynamic>? ?? const {},
        fallbackTitle: empty ?? 'Android bugreport',
        fallbackHeadline: empty ?? 'We need a full system bugreport',
        fallbackBody: empty ??
            'Android will create a ZIP with logs, installed apps, and crash data. '
                'You stay in control: you start it from Developer options and share it into this app.',
      ),
      iosSysdiagnose: GuideCopy.fromJson(
        json['ios_sysdiagnose'] as Map<String, dynamic>? ?? const {},
        fallbackTitle: empty ?? 'iOS sysdiagnose',
        fallbackHeadline: empty ?? 'Prepare the phone',
        fallbackBody: empty ??
            'Unlock the phone, connect to Wi‑Fi, and charge above 30%. '
                'Sysdiagnose is heavy on CPU and storage.',
      ),
      messageReport: MessageCopy.fromJson(
        json['message_report'] as Map<String, dynamic>? ?? const {},
        loose: loose,
      ),
      notifications: NotificationsCopy.fromJson(
        json['notifications'] as Map<String, dynamic>? ?? const {},
        loose: loose,
      ),
    );
  }
}

class TenantStringsI18n {
  TenantStringsI18n({
    required this.tagline,
    required this.enrollLead,
    required this.otpNotificationSubject,
    required this.otpNotificationBody,
  });

  final String tagline;
  final String enrollLead;
  final String otpNotificationSubject;
  final String otpNotificationBody;

  factory TenantStringsI18n.fromJson(Map<String, dynamic> json) {
    return TenantStringsI18n(
      tagline: json['tagline'] as String? ?? '',
      enrollLead: json['enroll_lead'] as String? ?? '',
      otpNotificationSubject: json['otp_notification_subject'] as String? ?? '',
      otpNotificationBody: json['otp_notification_body'] as String? ?? '',
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

  factory HomeCopy.fromJson(
    Map<String, dynamic> json, {
    bool loose = false,
  }) {
    String d(String? v, String fallback) =>
        loose ? (v ?? '') : (v ?? fallback);
    return HomeCopy(
      headline: d(json['headline'] as String?, 'What do you need to send?'),
      body: d(
        json['body'] as String?,
        'Your analyst asked for evidence from this phone. '
            'Follow the guided steps — we never collect anything without your consent.',
      ),
      androidTitle: d(json['android_title'] as String?, 'Android bugreport'),
      androidSubtitle: d(
        json['android_subtitle'] as String?,
        'System diagnostic ZIP via Developer options',
      ),
      iosTitle: d(json['ios_title'] as String?, 'iOS sysdiagnose'),
      iosSubtitle: d(
        json['ios_subtitle'] as String?,
        'Hardware button combo, then share the archive',
      ),
      messageTitle: d(json['message_title'] as String?, 'Suspicious message'),
      messageSubtitle: d(
        json['message_subtitle'] as String?,
        'Share, screenshot, or paste the message',
      ),
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

  factory ConsentCopy.fromJson(
    Map<String, dynamic> json, {
    bool loose = false,
  }) {
    String d(String? v, String fallback) =>
        loose ? (v ?? '') : (v ?? fallback);
    return ConsentCopy(
      title: d(json['title'] as String?, 'Consent before upload'),
      footer: d(
        json['footer'] as String?,
        'The file is encrypted on this phone to the case key before upload. '
            'You can review or discard later from My data.',
      ),
      checkbox: d(
        json['checkbox'] as String?,
        'I understand and consent to send this artefact',
      ),
      button: d(json['button'] as String?, 'Encrypt and upload'),
      disclosureAndroid: d(
        json['disclosure_android'] as String?,
        'A bugreport can include installed apps, system logs, Wi‑Fi networks, '
            'account identifiers, and sometimes location history.',
      ),
      disclosureIos: d(
        json['disclosure_ios'] as String?,
        'A sysdiagnose can include process lists, system logs, crash reports, '
            'network details, and other diagnostics collected by iOS.',
      ),
      disclosureMessage: d(
        json['disclosure_message'] as String?,
        'The message text, metadata you entered, and any screenshots will be '
            'sent encrypted to the analyst handling your case.',
      ),
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

  factory MyDataCopy.fromJson(
    Map<String, dynamic> json, {
    bool loose = false,
  }) {
    String d(String? v, String fallback) =>
        loose ? (v ?? '') : (v ?? fallback);
    return MyDataCopy(
      title: d(json['title'] as String?, 'My data'),
      deviceHeading: d(json['device_heading'] as String?, 'Device'),
      artefactsHeading:
          d(json['artefacts_heading'] as String?, 'Artefacts on this phone'),
      empty: d(json['empty'] as String?, 'Nothing queued yet.'),
      clearButton: d(json['clear_button'] as String?, 'Clear local enrollment'),
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

  factory MessageCopy.fromJson(
    Map<String, dynamic> json, {
    bool loose = false,
  }) {
    String d(String? v, String fallback) =>
        loose ? (v ?? '') : (v ?? fallback);
    return MessageCopy(
      title: d(json['title'] as String?, 'Suspicious message'),
      lead: d(
        json['lead'] as String?,
        'We never open links on this device. Confirm any URL we detect below.',
      ),
      button: d(json['button'] as String?, 'Continue to consent'),
    );
  }
}

class NotificationsCopy {
  NotificationsCopy({
    required this.encryptingTitle,
    required this.uploadingTitle,
    required this.uploadingProgressTitle,
    required this.completeTitle,
    required this.failedTitle,
    required this.completeBody,
    required this.failedBody,
  });

  final String encryptingTitle;
  final String uploadingTitle;
  final String uploadingProgressTitle;
  final String completeTitle;
  final String failedTitle;
  final String completeBody;
  final String failedBody;

  factory NotificationsCopy.fromJson(
    Map<String, dynamic> json, {
    bool loose = false,
  }) {
    String d(String? v, String fallback) =>
        loose ? (v ?? '') : (v ?? fallback);
    return NotificationsCopy(
      encryptingTitle: d(json['encrypting_title'] as String?, 'Encrypting…'),
      uploadingTitle: d(json['uploading_title'] as String?, 'Uploading…'),
      uploadingProgressTitle:
          d(json['uploading_progress_title'] as String?, 'Uploading… {pct}%'),
      completeTitle: d(json['complete_title'] as String?, 'Upload complete'),
      failedTitle: d(json['failed_title'] as String?, 'Upload failed'),
      completeBody:
          d(json['complete_body'] as String?, '{artefact} was sent to the lab.'),
      failedBody:
          d(json['failed_body'] as String?, '{artefact} could not be sent.'),
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
