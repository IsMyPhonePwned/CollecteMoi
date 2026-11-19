import 'dart:typed_data';

class EnvelopeFfi {
  static bool get isAvailable => false;
  static EnvelopeFfi? tryLoad() => null;

  ({Uint8List pk, Uint8List sk}) generateDeviceKeyPair() {
    throw UnsupportedError('FFI unavailable');
  }

  Uint8List seal({
    required Uint8List plaintext,
    required int artefact,
    required Uint8List casePk,
    required Uint8List deviceSk,
  }) {
    throw UnsupportedError('FFI unavailable');
  }
}
