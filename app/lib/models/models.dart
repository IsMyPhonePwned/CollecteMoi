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
  };

  factory EnrollmentSession.fromJson(Map<String, dynamic> json) {
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

  bool get canResumeConsent =>
      status == UploadJobStatus.pendingConsent ||
      status == UploadJobStatus.failed;

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
