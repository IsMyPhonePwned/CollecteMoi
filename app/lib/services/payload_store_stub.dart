import 'dart:typed_data';

/// Web / non-IO: payloads stay in the caller's memory map only.
class PayloadStore {
  Future<String?> write(String jobId, Uint8List bytes) async => null;

  Future<Uint8List?> read(String? path, {String? jobId}) async => null;

  Future<void> delete(String? path, {String? jobId}) async {}

  Future<bool> exists(String? path, {String? jobId}) async => false;
}
