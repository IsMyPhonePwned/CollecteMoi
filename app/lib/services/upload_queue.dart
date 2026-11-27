import 'dart:async';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:uuid/uuid.dart';

import '../config/tenant_config.dart';
import '../crypto/noise_wrap.dart';
import '../l10n/app_localizations.dart';
import '../models/models.dart';
import 'api_client.dart';
import 'envelope_service.dart';
import 'payload_store_stub.dart'
    if (dart.library.io) 'payload_store_io.dart';
import 'platform_bridge.dart';
import 'session_store.dart';

class UploadQueue extends ChangeNotifier {
  UploadQueue({
    required this._store,
    required this._api,
    required this._envelope,
    required TenantConfig tenant,
  }) : _tenant = tenant;

  final SessionStore _store;
  final ApiClient _api;
  final EnvelopeService _envelope;
  TenantConfig _tenant;
  final _uuid = const Uuid();
  final _payloadsDisk = PayloadStore();
  final _bridge = PlatformBridge();

  /// Hot cache — also mirrored to disk on native so process death is safe.
  final Map<String, Uint8List> _payloads = {};

  List<UploadJob> jobs = [];
  EnrollmentSession? session;
  bool busy = false;

  /// Job id currently encrypting/uploading (for home/My data banners).
  String? activeUploadJobId;

  /// Latest upload result for in-app SnackBar (cleared after shown).
  UploadCompletionNotice? latestCompletionNotice;

  static const chunkSize = 8 * 1024 * 1024;

  void clearCompletionNotice() {
    if (latestCompletionNotice == null) return;
    latestCompletionNotice = null;
    notifyListeners();
  }

  /// Refresh branded tenant (e.g. after tenant reload).
  void updateTenant(TenantConfig tenant) {
    _tenant = tenant;
  }

  NotificationsCopy get _notifications {
    final locale = AppLocalizations.resolveDeviceLocale(
      fallbackLanguageCode: _tenant.defaultLocale,
    );
    final base = _tenant.copy.notifications;
    final over = _tenant.copyI18n[locale.languageCode]?.notifications;
    String pick(String Function(NotificationsCopy n) get) {
      final o = over == null ? '' : get(over).trim();
      if (o.isNotEmpty) return o;
      return AppLocalizations.translate(get(base), locale);
    }

    return NotificationsCopy(
      encryptingTitle: pick((n) => n.encryptingTitle),
      uploadingTitle: pick((n) => n.uploadingTitle),
      uploadingProgressTitle: pick((n) => n.uploadingProgressTitle),
      completeTitle: pick((n) => n.completeTitle),
      failedTitle: pick((n) => n.failedTitle),
      completeBody: pick((n) => n.completeBody),
      failedBody: pick((n) => n.failedBody),
    );
  }

  String _trNotif(String value) => value;

  UploadJob? get activeUploadJob {
    final id = activeUploadJobId;
    if (id == null) return null;
    for (final j in jobs) {
      if (j.id == id) return j;
    }
    return null;
  }

  bool get hasBackgroundUpload =>
      busy ||
      jobs.any(
        (j) =>
            j.status == UploadJobStatus.encrypting ||
            j.status == UploadJobStatus.uploading,
      );

  Future<void> load() async {
    session = await _store.loadSession();
    jobs = await _store.loadJobs();
    await _api.refreshAppVersion();
    if (session != null) {
      _api.baseUrl = session!.serverBaseUrl;
      _envelope.labSealBaseUrl = session!.serverBaseUrl;
      _api.applySession(session);
      _envelope.applyClientIdentity(_api.identity);
    }
    // Drop any persisted message previews (body/sender snippets must not surface in UI).
    var cleared = false;
    for (final job in jobs) {
      if (job.summary.trim().isEmpty) continue;
      job.summary = '';
      cleared = true;
    }
    if (cleared) {
      await _persist();
    }
    notifyListeners();
  }

  Future<void> setSession(EnrollmentSession value) async {
    session = value;
    _api.baseUrl = value.serverBaseUrl;
    _envelope.labSealBaseUrl = value.serverBaseUrl;
    await _api.refreshAppVersion();
    _api.applySession(value);
    _envelope.applyClientIdentity(_api.identity);
    await _store.saveSession(value);
    notifyListeners();
  }

  /// Update the lab / product name shown in the app (home title, My data).
  Future<void> setDisplayName(String name) async {
    final current = session;
    if (current == null) return;
    final trimmed = name.trim();
    if (trimmed.isEmpty || trimmed == current.displayName) return;
    await setSession(current.copyWith(displayName: trimmed));
  }

  /// Update API / seal base URL before enrollment (e.g. Settings edit).
  /// Does not change an existing session's stored URL until re-enroll.
  void applyServerBaseUrl(String url) {
    final normalized = url.trim().replaceAll(RegExp(r'/$'), '');
    if (normalized.isEmpty) return;
    _api.baseUrl = normalized;
    _envelope.labSealBaseUrl = normalized;
  }

  Future<void> clearSession() async {
    session = null;
    _api.applySession(null);
    _envelope.applyClientIdentity(_api.identity);
    await _store.clearSession();
    notifyListeners();
  }

  Future<bool> hasPayload(String jobId) async {
    if (_payloads.containsKey(jobId)) return true;
    UploadJob? job;
    for (final j in jobs) {
      if (j.id == jobId) {
        job = j;
        break;
      }
    }
    return _payloadsDisk.exists(job?.payloadPath, jobId: jobId);
  }

  /// Read local plaintext payload (before encrypt/upload).
  Future<Uint8List?> payloadBytes(String jobId) async {
    final index = jobs.indexWhere((j) => j.id == jobId);
    if (index < 0) return _payloads[jobId];
    return _loadPlaintext(jobs[index]);
  }

  Future<UploadJob> enqueueBytes({
    required ArtefactKind artefact,
    required Uint8List bytes,
    String label = 'upload.bin',
  }) async {
    final id = _uuid.v4();
    final path = await _payloadsDisk.write(id, bytes);
    _payloads[id] = bytes;
    final job = UploadJob(
      id: id,
      artefact: artefact,
      localPath: label,
      createdAt: DateTime.now(),
      sizeBytes: bytes.length,
      status: UploadJobStatus.pendingConsent,
      payloadPath: path,
    );
    jobs = [...jobs, job];
    await _persist();
    notifyListeners();
    return job;
  }

  /// Reset a failed / interrupted job so consent can run again.
  Future<void> prepareRetry(String jobId) async {
    final index = jobs.indexWhere((j) => j.id == jobId);
    if (index < 0) return;
    final job = jobs[index];
    if (!await hasPayload(jobId)) {
      job.status = UploadJobStatus.failed;
      job.error =
          'Payload missing — re-select or re-share the file, then try again.';
      await _persist();
      notifyListeners();
      return;
    }
    job.status = UploadJobStatus.pendingConsent;
    job.error = null;
    job.blocked = false;
    job.progress = 0;
    job.uploadId = null;
    await _persist();
    notifyListeners();
  }

  /// One-tap resend: keeps consent, re-encrypts and uploads again.
  Future<void> resend(String jobId) async {
    final index = jobs.indexWhere((j) => j.id == jobId);
    if (index < 0) return;
    final job = jobs[index];
    if (busy) return;
    if (!await hasPayload(jobId)) {
      job.status = UploadJobStatus.failed;
      job.error =
          'Payload missing — re-select or re-share the file, then try again.';
      await _persist();
      notifyListeners();
      return;
    }
    job.status = UploadJobStatus.queued;
    job.error = null;
    job.blocked = false;
    job.progress = 0;
    job.uploadId = null;
    job.consentGiven = true;
    await _persist();
    notifyListeners();
    await consentAndUpload(jobId);
  }

  /// Resend every failed job that still has a local payload.
  Future<void> resendAllFailed() async {
    final ids = jobs
        .where((j) => j.canResendFast)
        .map((j) => j.id)
        .toList();
    for (final id in ids) {
      if (busy) {
        while (busy) {
          await Future<void>.delayed(const Duration(milliseconds: 200));
        }
      }
      await resend(id);
    }
  }

  Future<void> discardJob(String jobId) async {
    final index = jobs.indexWhere((j) => j.id == jobId);
    if (index < 0) return;
    final job = jobs[index];
    // Do not delete a job that is actively encrypting/uploading.
    if (job.id == activeUploadJobId ||
        job.status == UploadJobStatus.encrypting ||
        job.status == UploadJobStatus.uploading) {
      return;
    }
    _payloads.remove(jobId);
    await _payloadsDisk.delete(job.payloadPath, jobId: jobId);
    jobs = [...jobs]..removeAt(index);
    await _persist();
    notifyListeners();
  }

  /// Remove every failed local job (and its payload) from this phone.
  Future<int> discardAllFailed() async {
    final ids = jobs
        .where((j) => j.status == UploadJobStatus.failed)
        .map((j) => j.id)
        .toList();
    for (final id in ids) {
      await discardJob(id);
    }
    return ids.length;
  }

  Future<Uint8List?> _loadPlaintext(UploadJob job) async {
    final cached = _payloads[job.id];
    if (cached != null) return cached;
    final fromDisk =
        await _payloadsDisk.read(job.payloadPath, jobId: job.id);
    if (fromDisk != null) {
      _payloads[job.id] = fromDisk;
    }
    return fromDisk;
  }

  /// Consent accepted — start encrypt/upload without blocking the UI.
  /// Returns immediately; progress is visible on My data / home.
  void startConsentedUpload(String jobId) {
    if (busy) return;
    unawaited(consentAndUpload(jobId));
  }

  Future<void> consentAndUpload(String jobId) async {
    final session = this.session;
    if (session == null) {
      throw StateError('Not enrolled');
    }
    if (busy) return;
    final index = jobs.indexWhere((j) => j.id == jobId);
    if (index < 0) return;
    final job = jobs[index];
    final plaintext = await _loadPlaintext(job);
    if (plaintext == null) {
      job.status = UploadJobStatus.failed;
      job.error =
          'Payload missing — re-select or re-share the file. '
          '${kIsWeb ? "(Web keeps payloads in memory only.)" : "Expected a file under app documents/payloads."}';
      await _persist();
      notifyListeners();
      return;
    }

    job.consentGiven = true;
    job.blocked = false;
    job.status = UploadJobStatus.encrypting;
    job.error = null;
    job.progress = 0;
    job.uploadId = null;
    activeUploadJobId = jobId;
    busy = true;
    notifyListeners();

    final artefactLabel = AppLocalizations.trDevice(job.artefact.label);
    final notif = _notifications;
    await _bridge.startUploadForeground(
      title: _trNotif(notif.encryptingTitle),
      text: artefactLabel,
    );

    try {
      final sealed = await _envelope.sealBytes(
        plaintext: plaintext,
        artefact: job.artefact,
        casePublicKeyHex: session.casePublicKeyHex,
        devicePrivateKeyHex: session.devicePrivateKeyHex,
        devicePublicKeyHex: session.devicePublicKeyHex,
      );

      if (session.noiseKeyHex.isEmpty) {
        throw StateError('Session missing noise key — re-enroll');
      }
      final wrapped = await NoiseWrap.seal(
        plaintext: sealed.bytes,
        noiseKeyHex: session.noiseKeyHex,
      );
      final wrappedDigest = sha256.convert(wrapped).toString();

      job.status = UploadJobStatus.uploading;
      notifyListeners();
      await _bridge.updateUploadForeground(
        title: _trNotif(notif.uploadingTitle),
        text: artefactLabel,
      );

      await _uploadWithRetries(
        job: job,
        wrapped: wrapped,
        wrappedDigest: wrappedDigest,
        caseId: session.caseId,
        devicePublicKeyHex: session.devicePublicKeyHex,
        artefactLabel: artefactLabel,
        notif: notif,
      );

      job.status = UploadJobStatus.complete;
      job.blocked = false;
      job.progress = 1;
      job.error = null;
      _payloads.remove(jobId);
      await _payloadsDisk.delete(job.payloadPath, jobId: jobId);
      job.payloadPath = null;
      latestCompletionNotice = UploadCompletionNotice(
        success: true,
        artefactLabel: job.artefact.label,
      );
    } catch (e) {
      final banned = e is ApiException && e.isDeviceBanned;
      final rateLimited = e is ApiException && e.isDeviceRateLimited;
      job.status = UploadJobStatus.failed;
      job.blocked = banned;
      job.error = banned
          ? 'This phone was blocked by the lab. Contact your administrator, then tap Resend.'
          : rateLimited
          ? 'Upload limit reached (too many reports from this phone). Wait and try again later.'
          : _friendlyUploadError(e);
      latestCompletionNotice = UploadCompletionNotice(
        success: false,
        artefactLabel: job.artefact.label,
        error: job.error,
      );
    } finally {
      busy = false;
      activeUploadJobId = null;
      await _bridge.stopUploadForeground();
      final notice = latestCompletionNotice;
      if (notice != null) {
        final label = AppLocalizations.trDevice(notice.artefactLabel);
        await _bridge.notifyUploadFinished(
          title: _trNotif(
            notice.success ? notif.completeTitle : notif.failedTitle,
          ),
          text: _trNotif(
            notice.success ? notif.completeBody : notif.failedBody,
          ).replaceAll('{artefact}', label),
          success: notice.success,
        );
      }
      await _persist();
      notifyListeners();
    }
  }

  Future<void> _uploadWithRetries({
    required UploadJob job,
    required Uint8List wrapped,
    required String wrappedDigest,
    required String caseId,
    required String devicePublicKeyHex,
    required String artefactLabel,
    required NotificationsCopy notif,
  }) async {
    const maxAttempts = 3;
    Object? lastError;
    for (var attempt = 1; attempt <= maxAttempts; attempt++) {
      try {
        job.uploadId = null;
        job.progress = 0;
        notifyListeners();

        final totalSize = wrapped.length;
        final chunkCount = (totalSize / chunkSize).ceil().clamp(1, 1 << 30);
        final uploadId = await _api.createUpload(
          caseId: caseId,
          artefact: job.artefact,
          totalSize: totalSize,
          chunkSize: chunkSize,
          chunkCount: chunkCount,
          ciphertextSha256: wrappedDigest,
        );
        job.uploadId = uploadId;

        for (var n = 0; n < chunkCount; n++) {
          final start = n * chunkSize;
          final end = (start + chunkSize).clamp(0, totalSize);
          final chunk = wrapped.sublist(start, end);
          final digest = sha256.convert(chunk).toString();
          await _api.putChunk(
            uploadId: uploadId,
            index: n,
            bytes: chunk,
            sha256Hex: digest,
          );
          job.progress = (n + 1) / chunkCount;
          notifyListeners();
          final pct = ((job.progress) * 100).round();
          await _bridge.updateUploadForeground(
            title: _trNotif(notif.uploadingProgressTitle)
                .replaceAll('{pct}', '$pct'),
            text: artefactLabel,
          );
        }

        await _api.completeUpload(
          uploadId: uploadId,
          devicePublicKeyHex: devicePublicKeyHex,
        );
        return;
      } catch (e) {
        lastError = e;
        final retryable = e is ApiException
            ? e.isTransient
            : e.toString().toLowerCase().contains('socket') ||
                e.toString().toLowerCase().contains('timeout') ||
                e.toString().toLowerCase().contains('connection');
        if (e is ApiException && (e.isDeviceBanned || e.isDeviceRateLimited)) {
          rethrow;
        }
        if (!retryable || attempt >= maxAttempts) {
          rethrow;
        }
        final wait = Duration(seconds: attempt * 2);
        job.error = 'Connection problem — retrying (${attempt + 1}/$maxAttempts)…';
        notifyListeners();
        await Future<void>.delayed(wait);
        job.error = null;
      }
    }
    throw lastError ?? StateError('upload failed');
  }

  String _friendlyUploadError(Object e) {
    if (e is ApiException) {
      if (e.isDeviceBanned) {
        return 'This phone was blocked by the lab. Contact your administrator, then tap Resend.';
      }
      if (e.isDeviceRateLimited) {
        return 'Upload limit reached (too many reports from this phone). Wait and try again later.';
      }
      if (e.statusCode == 429) {
        return 'Too many requests — wait a moment, then tap Resend.';
      }
      if (e.isTransient) {
        return 'Network or server problem — tap Resend to try again.';
      }
      return 'Upload failed (HTTP ${e.statusCode}). Tap Resend to try again.';
    }
    final s = e.toString();
    if (s.toLowerCase().contains('socket') ||
        s.toLowerCase().contains('timeout') ||
        s.toLowerCase().contains('connection')) {
      return 'Network problem — tap Resend to try again.';
    }
    return 'Upload failed — tap Resend to try again.';
  }

  Future<void> _persist() => _store.saveJobs(jobs);
}

class UploadCompletionNotice {
  const UploadCompletionNotice({
    required this.success,
    required this.artefactLabel,
    this.error,
  });

  final bool success;
  final String artefactLabel;
  final String? error;
}
