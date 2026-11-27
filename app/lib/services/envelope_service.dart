import 'dart:convert';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;

import '../crypto/envelope_bridge.dart';
import '../models/models.dart';
import 'api_client.dart';

/// Seals artefacts as CollecteMoi `CM01` envelopes.
///
/// Prefer native FFI (`libcollectemoi_envelope`). On web (or when the dylib is
/// missing) fall back to the local API lab seal endpoint which runs the same
/// Rust implementation — enable with `--lab-seal true` on collectemoi-api.
class EnvelopeService {
  EnvelopeService({this.labSealBaseUrl});

  /// API base URL for `/v1/lab/seal` fallback (web / missing dylib).
  String? labSealBaseUrl;

  /// Mirrored from [ApiClient] so lab-seal packets carry the same device headers.
  ClientIdentity clientIdentity = const ClientIdentity();

  void applyClientIdentity(ClientIdentity identity) {
    clientIdentity = identity;
  }

  Future<SealedArtefact> sealBytes({
    required Uint8List plaintext,
    required ArtefactKind artefact,
    required String casePublicKeyHex,
    required String devicePrivateKeyHex,
    required String devicePublicKeyHex,
  }) async {
    final casePk = _hexDecode(casePublicKeyHex);
    final deviceSk = _hexDecode(devicePrivateKeyHex);
    if (casePk.isEmpty) {
      throw StateError(
        'Case public key missing — re-enroll after the API has generated keys',
      );
    }

    final ffi = EnvelopeFfi.tryLoad();
    late Uint8List sealed;
    if (ffi != null) {
      sealed = ffi.seal(
        plaintext: plaintext,
        artefact: _artefactCode(artefact),
        casePk: casePk,
        deviceSk: deviceSk,
      );
    } else {
      sealed = await _labSeal(
        plaintext: plaintext,
        artefact: artefact,
        casePublicKeyHex: casePublicKeyHex,
        devicePrivateKeyHex: devicePrivateKeyHex,
      );
    }

    if (sealed.length < 4 ||
        sealed[0] != 0x43 ||
        sealed[1] != 0x4d ||
        sealed[2] != 0x30 ||
        sealed[3] != 0x31) {
      throw StateError('seal did not produce CM01 magic');
    }

    return SealedArtefact(
      bytes: sealed,
      size: sealed.length,
      sha256Hex: sha256.convert(sealed).toString(),
    );
  }

  Future<Uint8List> _labSeal({
    required Uint8List plaintext,
    required ArtefactKind artefact,
    required String casePublicKeyHex,
    required String devicePrivateKeyHex,
  }) async {
    final base = labSealBaseUrl;
    if (base == null || base.isEmpty) {
      throw StateError(
        'Native envelope library not loaded and no labSealBaseUrl set. '
        'Build the workspace (`cargo build -p collectemoi-envelope`) '
        'or run the API with --lab-seal true for web.',
      );
    }
    final id = clientIdentity;
    final uaPlatform = id.platform.isEmpty ? 'unknown' : id.platform;
    final uaModel = id.model.isEmpty ? uaPlatform : id.model;
    final uaVersion = id.appVersion.isEmpty ? 'dev' : id.appVersion;
    final res = await http.post(
      Uri.parse('$base/v1/lab/seal'),
      headers: {
        'content-type': 'application/json',
        'user-agent': 'CollecteMoi/$uaVersion ($uaPlatform; $uaModel)',
        if ((id.deviceId ?? '').isNotEmpty) 'x-device-id': id.deviceId!,
        if (id.appVersion.isNotEmpty) 'x-app-version': id.appVersion,
        if (id.platform.isNotEmpty) 'x-device-platform': id.platform,
        if (id.model.isNotEmpty) 'x-device-model': id.model,
        if (id.osVersion.isNotEmpty) 'x-device-os': id.osVersion,
        if (id.manufacturer.isNotEmpty) 'x-device-manufacturer': id.manufacturer,
        if (id.brand.isNotEmpty) 'x-device-brand': id.brand,
        if (id.name.isNotEmpty) 'x-device-name': id.name,
      },
      body: jsonEncode({
        'plaintext_b64': base64Encode(plaintext),
        'artefact': artefact.apiValue,
        'case_public_key': casePublicKeyHex,
        'device_private_key': devicePrivateKeyHex,
      }),
    );
    if (res.statusCode < 200 || res.statusCode >= 300) {
      throw StateError('lab seal failed: ${res.statusCode} ${res.body}');
    }
    final body = jsonDecode(res.body) as Map<String, dynamic>;
    return base64Decode(body['envelope_b64'] as String);
  }

  int _artefactCode(ArtefactKind k) => switch (k) {
        ArtefactKind.androidBugreport => 0,
        ArtefactKind.iosSysdiagnose => 1,
        ArtefactKind.messageReport => 2,
      };
}

class SealedArtefact {
  SealedArtefact({
    required this.bytes,
    required this.size,
    required this.sha256Hex,
  });

  final Uint8List bytes;
  final int size;
  final String sha256Hex;
}

Uint8List _hexDecode(String hex) {
  final cleaned = hex.trim();
  if (cleaned.isEmpty) return Uint8List(0);
  if (cleaned.length.isOdd) {
    throw FormatException('odd hex length');
  }
  final out = Uint8List(cleaned.length ~/ 2);
  for (var i = 0; i < out.length; i++) {
    out[i] = int.parse(cleaned.substring(i * 2, i * 2 + 2), radix: 16);
  }
  return out;
}

// silence unused in some analyzer configs
bool get envelopeUsesFfiPreferentially => !kIsWeb && EnvelopeFfi.isAvailable;
