import 'dart:convert';

import 'dart:convert';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import '../crypto/noise_wrap.dart';
import '../models/models.dart';

class ApiClient {
  ApiClient({required this.baseUrl, http.Client? client})
    : _client = client ?? http.Client();

  String baseUrl;
  final http.Client _client;

  Uri _u(String path) => Uri.parse('$baseUrl$path');

  Future<Map<String, dynamic>> enrollStart({
    String? email,
    String? labKey,
  }) async {
    final res = await _client.post(
      _u('/v1/enroll/start'),
      headers: {'content-type': 'application/json'},
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
    required String platform,
    required String model,
    required String osVersion,
  }) async {
    final res = await _client.post(
      _u('/v1/enroll/confirm'),
      headers: {'content-type': 'application/json'},
      body: jsonEncode({
        'challenge_id': challengeId,
        'code': code,
        'device_public_key': devicePublicKeyHex,
        'noise_key': noiseKeyHex,
        'platform': platform,
        'model': model,
        'os_version': osVersion,
      }),
    );
    _ensureOk(res);
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  Future<Map<String, dynamic>> enroll({
    required String token,
    required String devicePublicKeyHex,
    required String noiseKeyHex,
    required String platform,
    required String model,
    required String osVersion,
  }) async {
    final res = await _client.post(
      _u('/v1/enroll'),
      headers: {'content-type': 'application/json'},
      body: jsonEncode({
        'token': token,
        'device_public_key': devicePublicKeyHex,
        'noise_key': noiseKeyHex,
        'platform': platform,
        'model': model,
        'os_version': osVersion,
      }),
    );
    _ensureOk(res);
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  Future<Map<String, dynamic>> tenantManifest() async {
    final res = await _client.get(_u('/v1/tenant/manifest'));
    _ensureOk(res);
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  Future<List<CollectionRequest>> requests() async {
    final res = await _client.get(_u('/v1/requests'));
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
      headers: {'content-type': 'application/json'},
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
      headers: {
        'content-type': 'application/octet-stream',
        'digest': 'sha-256=$sha256Hex',
      },
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
      headers: {'content-type': 'application/json'},
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
    final res = await _client.post(
      _u('/v1/client/submissions'),
      headers: {
        'content-type': 'application/octet-stream',
        'x-device-id': deviceId,
      },
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
      headers: {'content-type': 'application/json'},
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

  @override
  String toString() => 'ApiException($statusCode): $body';
}
