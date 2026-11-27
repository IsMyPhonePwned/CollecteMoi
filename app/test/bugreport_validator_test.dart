import 'dart:convert';
import 'dart:typed_data';

import 'package:archive/archive.dart';
import 'package:collectemoi_app/services/bugreport_validator.dart';
import 'package:flutter_test/flutter_test.dart';

Uint8List _zipWith(Map<String, List<int>> files) {
  final archive = Archive();
  for (final e in files.entries) {
    archive.addFile(ArchiveFile.bytes(e.key, e.value));
  }
  return Uint8List.fromList(ZipEncoder().encode(archive));
}

void main() {
  test('accepts classic main_entry + bugreport-*.txt', () {
    final bytes = _zipWith({
      'main_entry.txt': utf8.encode('bugreport-pixel-2024.txt\n'),
      'bugreport-pixel-2024.txt': utf8.encode(
        '== dumpstate: 2024-01-01 00:00:00\nBuild fingerprint: test\n',
      ),
    });
    final v = validateAndroidBugreport(bytes);
    expect(v.ok, isTrue, reason: v.messages.join('\n'));
  });

  test('accepts bugreport text nested under a folder', () {
    final bytes = _zipWith({
      'report/main_entry.txt': utf8.encode('bugreport-nested.txt\n'),
      'report/bugreport-nested.txt': utf8.encode(
        '== dumpstate: 2024-01-01 00:00:00\n',
      ),
    });
    final v = validateAndroidBugreport(bytes);
    expect(v.ok, isTrue, reason: v.messages.join('\n'));
  });

  test('accepts modern dumpstate_board + version without classic name', () {
    final bytes = _zipWith({
      'main_entry.txt': utf8.encode('dumpstate_board.txt\n'),
      'dumpstate_board.txt': utf8.encode(
        '== dumpstate: 2024-06-01\nBuild fingerprint: oem/device\n',
      ),
      'version.txt': utf8.encode('1.0\n'),
      'proto/status.pb': [0x00],
    });
    final v = validateAndroidBugreport(bytes);
    expect(v.ok, isTrue, reason: v.messages.join('\n'));
  });

  test('accepts plain dumpstate text', () {
    final text = utf8.encode(
      '== dumpstate: 2024-01-01 12:00:00\nBuild fingerprint: x\n',
    );
    final v = validateAndroidBugreport(Uint8List.fromList(text));
    expect(v.ok, isTrue, reason: v.messages.join('\n'));
  });

  test('rejects random zip', () {
    final bytes = _zipWith({'readme.txt': utf8.encode('hello')});
    final v = validateAndroidBugreport(bytes);
    expect(v.ok, isFalse);
  });
}
