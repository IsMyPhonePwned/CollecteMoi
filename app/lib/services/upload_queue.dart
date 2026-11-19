import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:uuid/uuid.dart';

import '../crypto/noise_wrap.dart';
import '../models/models.dart';
import 'api_client.dart';
import 'envelope_service.dart';
import 'payload_store_stub.dart'
    if (dart.library.io) 'payload_store_io.dart';
import 'session_store.dart';

class UploadQueue extends ChangeNotifier {
  UploadQueue({
    required this._store,
    required this._api,
    required this._envelope,
  });

  final SessionStore _store;
  final ApiClient _api;
  final EnvelopeService _envelope;
  final _uuid = const Uuid();
  final _payloadsDisk = PayloadStore();

  /// Hot cache — also mirrored to disk on native so process death is safe.
  final Map<String, Uint8List> _payloads = {};

  List<UploadJob> jobs = [];
  EnrollmentSession? session;
  bool busy = false;

  static const chunkSize = 8 * 1024 * 1024;

  Future<void> load() async {
    session = await _store.loadSession();
    jobs = await _store.loadJobs();
    if (session != null) {
      _api.baseUrl = session!.serverBaseUrl;
      _envelope.labSealBaseUrl = session!.serverBaseUrl;
    }
    notifyListeners();
  }

  Future<void> setSession(EnrollmentSession value) async {
    session = value;
    _api.baseUrl = value.serverBaseUrl;
    _envelope.labSealBaseUrl = value.serverBaseUrl;
    await _store.saveSession(value);
    notifyListeners();
  }

  Future<void> clearSession() async {
    session = null;
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
    job.progress = 0;
    job.uploadId = null;
    await _persist();
    notifyListeners();
  }

  Future<void> discardJob(String jobId) async {
    final index = jobs.indexWhere((j) => j.id == jobId);
    if (index < 0) return;
    final job = jobs[index];
    _payloads.remove(jobId);
    await _payloadsDisk.delete(job.payloadPath, jobId: jobId);
    jobs = [...jobs]..removeAt(index);
    await _persist();
    notifyListeners();
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

  Future<void> consentAndUpload(String jobId) async {
    final session = this.session;
    if (session == null) {
      throw StateError('Not enrolled');
    }
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

    job.status = UploadJobStatus.encrypting;
    job.error = null;
    notifyListeners();

    busy = true;
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

      final totalSize = wrapped.length;
      final chunkCount = (totalSize / chunkSize).ceil().clamp(1, 1 << 30);
      final uploadId = await _api.createUpload(
        caseId: session.caseId,
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
      }

      await _api.completeUpload(
        uploadId: uploadId,
        devicePublicKeyHex: session.devicePublicKeyHex,
      );
      job.status = UploadJobStatus.complete;
      job.progress = 1;
      _payloads.remove(jobId);
      await _payloadsDisk.delete(job.payloadPath, jobId: jobId);
      job.payloadPath = null;
    } catch (e) {
      job.status = UploadJobStatus.failed;
      job.error = e.toString();
    } finally {
      busy = false;
      await _persist();
      notifyListeners();
    }
  }

  Future<void> _persist() => _store.saveJobs(jobs);
}
