import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';

/// Loads `libcollectemoi_envelope` (`cdylib` from the Cargo workspace).
class EnvelopeFfi {
  EnvelopeFfi._(DynamicLibrary lib)
      : _genDevice = lib.lookupFunction<_KeygenNative, _KeygenDart>(
          'cm_generate_device_keypair',
        ),
        _seal = lib.lookupFunction<_SealNative, _SealDart>('cm_seal'),
        _free = lib.lookupFunction<_FreeNative, _FreeDart>('cm_free');

  final _KeygenDart _genDevice;
  final _SealDart _seal;
  final _FreeDart _free;

  static EnvelopeFfi? _instance;

  static bool get isAvailable => tryLoad() != null;

  static EnvelopeFfi? tryLoad() {
    if (kIsWeb) return null;
    if (_instance != null) return _instance;
    try {
      _instance = EnvelopeFfi._(_openLibrary());
      return _instance;
    } catch (_) {
      return null;
    }
  }

  static DynamicLibrary _openLibrary() {
    if (Platform.isMacOS) {
      for (final path in _macCandidates) {
        try {
          return DynamicLibrary.open(path);
        } catch (_) {}
      }
    }
    if (Platform.isLinux) {
      for (final path in [
        'libcollectemoi_envelope.so',
        '../target/debug/libcollectemoi_envelope.so',
        '../../target/debug/libcollectemoi_envelope.so',
      ]) {
        try {
          return DynamicLibrary.open(path);
        } catch (_) {}
      }
    }
    if (Platform.isAndroid) {
      return DynamicLibrary.open('libcollectemoi_envelope.so');
    }
    if (Platform.isIOS) {
      return DynamicLibrary.process();
    }
    if (Platform.isWindows) {
      return DynamicLibrary.open('collectemoi_envelope.dll');
    }
    throw UnsupportedError('envelope native library not found');
  }

  static final _macCandidates = <String>[
    'libcollectemoi_envelope.dylib',
    '../target/debug/libcollectemoi_envelope.dylib',
    '../../target/debug/libcollectemoi_envelope.dylib',
    '../../../target/debug/libcollectemoi_envelope.dylib',
    // Workspace-relative when running from app/
    '${Directory.current.path}/../target/debug/libcollectemoi_envelope.dylib',
  ];

  ({Uint8List pk, Uint8List sk}) generateDeviceKeyPair() {
    final pkPtr = calloc<Pointer<Uint8>>();
    final pkLen = calloc<Size>();
    final skPtr = calloc<Pointer<Uint8>>();
    final skLen = calloc<Size>();
    try {
      final rc = _genDevice(pkPtr, pkLen, skPtr, skLen);
      if (rc != 0) throw StateError('cm_generate_device_keypair failed');
      final pk = Uint8List.fromList(pkPtr.value.asTypedList(pkLen.value));
      final sk = Uint8List.fromList(skPtr.value.asTypedList(skLen.value));
      _free(pkPtr.value, pkLen.value);
      _free(skPtr.value, skLen.value);
      return (pk: pk, sk: sk);
    } finally {
      calloc.free(pkPtr);
      calloc.free(pkLen);
      calloc.free(skPtr);
      calloc.free(skLen);
    }
  }

  Uint8List seal({
    required Uint8List plaintext,
    required int artefact,
    required Uint8List casePk,
    required Uint8List deviceSk,
  }) {
    final pt = _toNative(plaintext);
    final cpk = _toNative(casePk);
    final dsk = _toNative(deviceSk);
    final outPtr = calloc<Pointer<Uint8>>();
    final outLen = calloc<Size>();
    try {
      final rc = _seal(
        pt,
        plaintext.length,
        artefact,
        cpk,
        casePk.length,
        dsk,
        deviceSk.length,
        outPtr,
        outLen,
      );
      if (rc != 0) throw StateError('cm_seal failed');
      final result = Uint8List.fromList(outPtr.value.asTypedList(outLen.value));
      _free(outPtr.value, outLen.value);
      return result;
    } finally {
      calloc.free(pt);
      calloc.free(cpk);
      calloc.free(dsk);
      calloc.free(outPtr);
      calloc.free(outLen);
    }
  }

  Pointer<Uint8> _toNative(Uint8List data) {
    final p = calloc<Uint8>(data.length);
    p.asTypedList(data.length).setAll(0, data);
    return p;
  }
}

typedef _KeygenNative = Int32 Function(
  Pointer<Pointer<Uint8>>,
  Pointer<Size>,
  Pointer<Pointer<Uint8>>,
  Pointer<Size>,
);
typedef _KeygenDart = int Function(
  Pointer<Pointer<Uint8>>,
  Pointer<Size>,
  Pointer<Pointer<Uint8>>,
  Pointer<Size>,
);

typedef _SealNative = Int32 Function(
  Pointer<Uint8>,
  Size,
  Uint8,
  Pointer<Uint8>,
  Size,
  Pointer<Uint8>,
  Size,
  Pointer<Pointer<Uint8>>,
  Pointer<Size>,
);
typedef _SealDart = int Function(
  Pointer<Uint8>,
  int,
  int,
  Pointer<Uint8>,
  int,
  Pointer<Uint8>,
  int,
  Pointer<Pointer<Uint8>>,
  Pointer<Size>,
);

typedef _FreeNative = Void Function(Pointer<Uint8>, Size);
typedef _FreeDart = void Function(Pointer<Uint8>, int);
