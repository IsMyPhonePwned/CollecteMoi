import 'dart:math';
import 'dart:typed_data';

import 'package:cryptography/cryptography.dart';

/// Per-client ChaCha20-Poly1305 noise wrap (`CMNK` || nonce || ciphertext||tag).
class NoiseWrap {
  static const magic = [0x43, 0x4d, 0x4e, 0x4b]; // CMNK
  static const keyLen = 32;
  static const nonceLen = 12;

  static String generateKeyHex() {
    final rnd = Random.secure();
    final bytes = Uint8List.fromList(
      List<int>.generate(keyLen, (_) => rnd.nextInt(256)),
    );
    return _hex(bytes);
  }

  static Future<Uint8List> seal({
    required Uint8List plaintext,
    required String noiseKeyHex,
  }) async {
    final keyBytes = _fromHex(noiseKeyHex);
    if (keyBytes.length != keyLen) {
      throw ArgumentError('noise key must be $keyLen bytes');
    }
    final nonce = Uint8List.fromList(
      List<int>.generate(nonceLen, (_) => Random.secure().nextInt(256)),
    );
    final algo = Chacha20.poly1305Aead();
    final secretKey = SecretKey(keyBytes);
    final box = await algo.encrypt(
      plaintext,
      secretKey: secretKey,
      nonce: nonce,
    );
    final out = BytesBuilder(copy: false)
      ..add(magic)
      ..add(nonce)
      ..add(box.cipherText)
      ..add(box.mac.bytes);
    return out.toBytes();
  }

  static String _hex(Uint8List bytes) =>
      bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

  static Uint8List _fromHex(String hex) {
    final cleaned = hex.trim();
    if (cleaned.length.isOdd) {
      throw FormatException('odd hex length');
    }
    final out = Uint8List(cleaned.length ~/ 2);
    for (var i = 0; i < out.length; i++) {
      out[i] = int.parse(cleaned.substring(i * 2, i * 2 + 2), radix: 16);
    }
    return out;
  }
}
