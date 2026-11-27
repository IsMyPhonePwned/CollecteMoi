import 'package:flutter/foundation.dart';

enum ArtefactKind {
  androidBugreport,
  iosSysdiagnose,
  messageReport;

  String get apiValue => switch (this) {
    ArtefactKind.androidBugreport => 'android_bugreport',
    ArtefactKind.iosSysdiagnose => 'ios_sysdiagnose',
    ArtefactKind.messageReport => 'message_report',
  };

  String get label => switch (this) {
    ArtefactKind.androidBugreport => 'Android bugreport',
    ArtefactKind.iosSysdiagnose => 'iOS sysdiagnose',
    ArtefactKind.messageReport => 'Suspicious message',
  };

  /// Hide the other phone OS collector so Android users never see sysdiagnose
  /// and iPhone users never see bugreport. Web/desktop keep both for lab use.
  bool get availableOnThisDevice => switch (this) {
    ArtefactKind.androidBugreport =>
      kIsWeb || defaultTargetPlatform == TargetPlatform.android,
    ArtefactKind.iosSysdiagnose =>
      kIsWeb || defaultTargetPlatform == TargetPlatform.iOS,
    ArtefactKind.messageReport => true,
  };
}

enum UploadJobStatus {
  pendingConsent,
  queued,
  encrypting,
  uploading,
  complete,
  failed,
}

class CollectionRequest {
  CollectionRequest({
    required this.id,
    required this.caseId,
    required this.title,
    required this.requestedArtefacts,
    this.deadline,
  });

  final String id;
  final String caseId;
  final String title;
  final List<ArtefactKind> requestedArtefacts;
  final DateTime? deadline;

  factory CollectionRequest.fromJson(Map<String, dynamic> json) {
    return CollectionRequest(
      id: json['id'] as String,
      caseId: json['case_id'] as String,
      title: json['title'] as String? ?? 'Collection request',
      requestedArtefacts: ((json['artefacts'] as List<dynamic>?) ?? [])
          .map((e) => _parseArtefact(e as String))
          .whereType<ArtefactKind>()
          .toList(),
      deadline: json['deadline'] != null
          ? DateTime.tryParse(json['deadline'] as String)
          : null,
    );
  }
}

ArtefactKind? _parseArtefact(String value) => switch (value) {
  'android_bugreport' => ArtefactKind.androidBugreport,
  'ios_sysdiagnose' => ArtefactKind.iosSysdiagnose,
  'message_report' => ArtefactKind.messageReport,
  _ => null,
};

class EnrollmentSession {
  EnrollmentSession({
    required this.deviceId,
    required this.caseId,
    required this.devicePublicKeyHex,
    required this.devicePrivateKeyHex,
    required this.casePublicKeyHex,
    required this.serverBaseUrl,
    required this.displayName,
    required this.noiseKeyHex,
    this.noiseKeyFingerprint,
    this.email,
    this.companyId,
    this.companyName,
    this.devicePlatform,
    this.deviceModel,
    this.deviceOsVersion,
    this.deviceManufacturer,
    this.deviceBrand,
    this.deviceName,
    this.deviceDetails = const {},
    this.appVersion,
  });

  final String deviceId;
  final String caseId;
  final String devicePublicKeyHex;
  final String devicePrivateKeyHex;
  final String casePublicKeyHex;
  final String serverBaseUrl;
  final String displayName;

  /// ChaCha20-Poly1305 noise key exchanged at enroll (32 bytes hex).
  final String noiseKeyHex;
  final String? noiseKeyFingerprint;
  final String? email;
  final String? companyId;

  /// Human company name from the server (enrollment / branding).
  final String? companyName;

  /// Captured at enroll from the phone (also sent to the server).
  final String? devicePlatform;
  final String? deviceModel;
  final String? deviceOsVersion;
  final String? deviceManufacturer;
  final String? deviceBrand;
  final String? deviceName;
  final Map<String, String> deviceDetails;

  /// App `version+build` reported on every API packet (e.g. `0.1.0+1`).
  final String? appVersion;

  EnrollmentSession copyWith({
    String? displayName,
    String? serverBaseUrl,
    String? appVersion,
  }) {
    return EnrollmentSession(
      deviceId: deviceId,
      caseId: caseId,
      devicePublicKeyHex: devicePublicKeyHex,
      devicePrivateKeyHex: devicePrivateKeyHex,
      casePublicKeyHex: casePublicKeyHex,
      serverBaseUrl: serverBaseUrl ?? this.serverBaseUrl,
      displayName: displayName ?? this.displayName,
      noiseKeyHex: noiseKeyHex,
      noiseKeyFingerprint: noiseKeyFingerprint,
      email: email,
      companyId: companyId,
      companyName: companyName,
      devicePlatform: devicePlatform,
      deviceModel: deviceModel,
      deviceOsVersion: deviceOsVersion,
      deviceManufacturer: deviceManufacturer,
      deviceBrand: deviceBrand,
      deviceName: deviceName,
      deviceDetails: deviceDetails,
      appVersion: appVersion ?? this.appVersion,
    );
  }

  bool get hasDeviceInfo =>
      (deviceModel?.isNotEmpty ?? false) ||
      (deviceOsVersion?.isNotEmpty ?? false) ||
      (devicePlatform?.isNotEmpty ?? false);

  String get deviceSummary {
    final parts = <String>[
      if ((deviceManufacturer ?? '').isNotEmpty &&
          !(deviceModel ?? '')
              .toLowerCase()
              .startsWith(deviceManufacturer!.toLowerCase()))
        deviceManufacturer!,
      if ((deviceModel ?? '').isNotEmpty) deviceModel!,
      if ((deviceOsVersion ?? '').isNotEmpty) deviceOsVersion!,
    ];
    if (parts.isNotEmpty) return parts.join(' · ');
    return devicePlatform ?? '';
  }

  Map<String, dynamic> toJson() => {
    'device_id': deviceId,
    'case_id': caseId,
    'device_public_key_hex': devicePublicKeyHex,
    'device_private_key_hex': devicePrivateKeyHex,
    'case_public_key_hex': casePublicKeyHex,
    'server_base_url': serverBaseUrl,
    'display_name': displayName,
    'noise_key_hex': noiseKeyHex,
    if (noiseKeyFingerprint != null)
      'noise_key_fingerprint': noiseKeyFingerprint,
    if (email != null) 'email': email,
    if (companyId != null) 'company_id': companyId,
    if (companyName != null) 'company_name': companyName,
    if (devicePlatform != null) 'device_platform': devicePlatform,
    if (deviceModel != null) 'device_model': deviceModel,
    if (deviceOsVersion != null) 'device_os_version': deviceOsVersion,
    if (deviceManufacturer != null) 'device_manufacturer': deviceManufacturer,
    if (deviceBrand != null) 'device_brand': deviceBrand,
    if (deviceName != null) 'device_name': deviceName,
    if (deviceDetails.isNotEmpty) 'device_details': deviceDetails,
    if (appVersion != null) 'app_version': appVersion,
  };

  factory EnrollmentSession.fromJson(Map<String, dynamic> json) {
    final rawDetails = json['device_details'];
    final details = <String, String>{};
    if (rawDetails is Map) {
      for (final e in rawDetails.entries) {
        final v = e.value?.toString().trim() ?? '';
        if (v.isNotEmpty) details[e.key.toString()] = v;
      }
    }
    return EnrollmentSession(
      deviceId: json['device_id'] as String,
      caseId: json['case_id'] as String,
      devicePublicKeyHex: json['device_public_key_hex'] as String,
      devicePrivateKeyHex: json['device_private_key_hex'] as String,
      casePublicKeyHex: json['case_public_key_hex'] as String,
      serverBaseUrl: json['server_base_url'] as String,
      displayName: json['display_name'] as String? ?? 'CollecteMoi',
      noiseKeyHex: json['noise_key_hex'] as String? ?? '',
      noiseKeyFingerprint: json['noise_key_fingerprint'] as String?,
      email: json['email'] as String?,
      companyId: json['company_id'] as String?,
      companyName: json['company_name'] as String?,
      devicePlatform: json['device_platform'] as String?,
      deviceModel: json['device_model'] as String?,
      deviceOsVersion: json['device_os_version'] as String?,
      deviceManufacturer: json['device_manufacturer'] as String?,
      deviceBrand: json['device_brand'] as String?,
      deviceName: json['device_name'] as String?,
      deviceDetails: details,
      appVersion: json['app_version'] as String?,
    );
  }
}

class UploadJob {
  UploadJob({
    required this.id,
    required this.artefact,
    required this.localPath,
    required this.createdAt,
    this.status = UploadJobStatus.pendingConsent,
    this.sizeBytes,
    this.uploadId,
    this.error,
    this.progress = 0,
    this.payloadPath,
    this.consentGiven = false,
    this.blocked = false,
    this.summary = '',
  });

  final String id;
  final ArtefactKind artefact;
  final String localPath;
  final DateTime createdAt;
  UploadJobStatus status;
  int? sizeBytes;
  String? uploadId;
  String? error;
  double progress;

  /// Absolute path to durable plaintext under app documents (native only).
  String? payloadPath;

  /// User already accepted consent once — resend can skip the consent screen.
  bool consentGiven;

  /// Server rejected this device (banned). Resend kept available after unban.
  bool blocked;

  /// Legacy field; kept for job JSON compatibility (not shown in UI).
  String summary;

  bool get canResumeConsent =>
      status == UploadJobStatus.pendingConsent ||
      status == UploadJobStatus.failed;

  /// One-tap resend without going through consent again.
  bool get canResendFast =>
      status == UploadJobStatus.failed && consentGiven;

  Map<String, dynamic> toJson() => {
    'id': id,
    'artefact': artefact.apiValue,
    'local_path': localPath,
    'created_at': createdAt.toIso8601String(),
    'status': status.name,
    'size_bytes': sizeBytes,
    'upload_id': uploadId,
    'error': error,
    'progress': progress,
    if (payloadPath != null) 'payload_path': payloadPath,
    'consent_given': consentGiven,
    'blocked': blocked,
    if (summary.isNotEmpty) 'summary': summary,
  };

  factory UploadJob.fromJson(Map<String, dynamic> json) {
    return UploadJob(
      id: json['id'] as String,
      artefact: _parseArtefact(json['artefact'] as String)!,
      localPath: json['local_path'] as String,
      createdAt: DateTime.parse(json['created_at'] as String),
      status: UploadJobStatus.values.firstWhere(
        (e) => e.name == json['status'],
        orElse: () => UploadJobStatus.queued,
      ),
      sizeBytes: json['size_bytes'] as int?,
      uploadId: json['upload_id'] as String?,
      error: json['error'] as String?,
      progress: (json['progress'] as num?)?.toDouble() ?? 0,
      payloadPath: json['payload_path'] as String?,
      consentGiven: json['consent_given'] as bool? ?? false,
      blocked: json['blocked'] as bool? ?? false,
      summary: (json['summary'] as String?)?.trim() ?? '',
    );
  }
}

class ServerSubmission {
  const ServerSubmission({
    required this.uploadId,
    required this.artefact,
    required this.sizeBytes,
    required this.submittedAt,
    required this.status,
  });

  final String uploadId;
  final ArtefactKind artefact;
  final int sizeBytes;
  final DateTime submittedAt;
  final String status;

  factory ServerSubmission.fromJson(Map<String, dynamic> json) {
    final timestamp = json['submitted_at'].toString();
    final epochSeconds = int.tryParse(timestamp);
    final submittedAt = epochSeconds == null
        ? DateTime.parse(timestamp).toLocal()
        : DateTime.fromMillisecondsSinceEpoch(
            epochSeconds * 1000,
            isUtc: true,
          ).toLocal();
    return ServerSubmission(
      uploadId: json['upload_id'] as String,
      artefact:
          _parseArtefact(json['artefact'] as String) ??
          ArtefactKind.androidBugreport,
      sizeBytes: (json['size_bytes'] as num?)?.toInt() ?? 0,
      submittedAt: submittedAt,
      status: json['status'] as String? ?? 'pending',
    );
  }
}

class MessageReportDraft {
  MessageReportDraft({
    this.channel = 'other',
    this.sender = '',
    this.bodyText = '',
    this.userNote = '',
    this.clickedLink = false,
    this.replied = false,
    this.attachmentPaths = const [],
  });

  String channel;
  String sender;
  String bodyText;
  String userNote;
  bool clickedLink;
  bool replied;
  List<String> attachmentPaths;
}
