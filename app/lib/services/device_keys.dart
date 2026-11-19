import 'dart:math';
import 'dart:typed_data';

import 'package:pointycastle/export.dart';

import '../crypto/envelope_bridge.dart';

class SoftDeviceKeyPair {
  SoftDeviceKeyPair({required this.privateKeyHex, required this.publicKeyHex});

  final String privateKeyHex;
  final String publicKeyHex;

  /// Real P-256 keys: native FFI when available, otherwise PointyCastle (web).
  static SoftDeviceKeyPair generate() {
    final ffi = EnvelopeFfi.tryLoad();
    if (ffi != null) {
      final pair = ffi.generateDeviceKeyPair();
      return SoftDeviceKeyPair(
        privateKeyHex: _hex(pair.sk),
        publicKeyHex: _hex(pair.pk),
      );
    }
    return _generatePointyCastle();
  }

  static SoftDeviceKeyPair _generatePointyCastle() {
    final domain = ECDomainParameters('secp256r1');
    final secureRandom = FortunaRandom()
      ..seed(
        KeyParameter(
          Uint8List.fromList(
            List<int>.generate(32, (_) => Random.secure().nextInt(256)),
          ),
        ),
      );
    final generator = ECKeyGenerator()
      ..init(ParametersWithRandom(ECKeyGeneratorParameters(domain), secureRandom));
    final pair = generator.generateKeyPair();
    final ECPrivateKey sk = pair.privateKey;
    final ECPublicKey pk = pair.publicKey;

    final d = _bigIntToFixed(sk.d!, 32);
    final qx = _bigIntToFixed(pk.Q!.x!.toBigInteger()!, 32);
    final qy = _bigIntToFixed(pk.Q!.y!.toBigInteger()!, 32);
    final uncompressed = Uint8List(65);
    uncompressed[0] = 0x04;
    uncompressed.setRange(1, 33, qx);
    uncompressed.setRange(33, 65, qy);

    return SoftDeviceKeyPair(
      privateKeyHex: _hex(d),
      publicKeyHex: _hex(uncompressed),
    );
  }
}

Uint8List _bigIntToFixed(BigInt value, int length) {
  var hex = value.toRadixString(16);
  if (hex.length.isOdd) hex = '0$hex';
  final bytes = Uint8List(hex.length ~/ 2);
  for (var i = 0; i < bytes.length; i++) {
    bytes[i] = int.parse(hex.substring(i * 2, i * 2 + 2), radix: 16);
  }
  if (bytes.length == length) return bytes;
  if (bytes.length > length) return bytes.sublist(bytes.length - length);
  final out = Uint8List(length);
  out.setRange(length - bytes.length, length, bytes);
  return out;
}

String _hex(Uint8List bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
