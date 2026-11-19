import 'dart:convert';
import 'dart:typed_data';

import 'package:archive/archive.dart';

class BugreportValidation {
  BugreportValidation({
    required this.ok,
    required this.messages,
    this.sizeBytes,
    this.captureHint,
  });

  final bool ok;
  final List<String> messages;
  final int? sizeBytes;
  final String? captureHint;
}

/// Client-side checks before consent (architecture § Android validation).
BugreportValidation validateAndroidBugreport(Uint8List bytes) {
  final messages = <String>[];
  if (bytes.length < 4) {
    return BugreportValidation(ok: false, messages: ['File too small']);
  }

  final isZip = bytes[0] == 0x50 && bytes[1] == 0x4b;
  if (!isZip) {
    // Allow plain text dumpstate for lab stubs.
    final head = utf8.decode(bytes.take(200).toList(), allowMalformed: true);
    if (head.contains('== dumpstate:') || head.contains('Bugreport')) {
      messages.add('Plain dumpstate text (not a ZIP) — accepted for lab');
      return BugreportValidation(
        ok: true,
        messages: messages,
        sizeBytes: bytes.length,
      );
    }
    return BugreportValidation(
      ok: false,
      messages: ['Not a ZIP and not a dumpstate text header'],
      sizeBytes: bytes.length,
    );
  }

  try {
    final archive = ZipDecoder().decodeBytes(bytes, verify: true);
    final names = archive.map((f) => f.name).toList();
    final hasMain = names.any((n) => n.endsWith('main_entry.txt') || n == 'main_entry.txt');
    if (!hasMain) {
      messages.add('ZIP missing main_entry.txt');
    } else {
      messages.add('Found main_entry.txt');
    }

    final hasBugreportTxt = names.any(
      (n) =>
          n.contains('bugreport') && (n.endsWith('.txt') || n.endsWith('.txt.gz')),
    );
    if (!hasBugreportTxt) {
      messages.add('No bugreport-*.txt entry found');
    } else {
      messages.add('Found bugreport text entry');
    }

    String? captureHint;
    final main = archive.firstWhere(
      (f) => f.name.endsWith('main_entry.txt'),
      orElse: () => archive.first,
    );
    if (main.name.endsWith('main_entry.txt')) {
      final content = utf8.decode(main.content as List<int>, allowMalformed: true).trim();
      captureHint = content.split('\n').first;
      final named = archive.files.where((f) => f.name.endsWith(content) || f.name.contains(content));
      if (named.isEmpty && content.isNotEmpty) {
        messages.add('main_entry names "$content" but that file is absent');
      }
    }

    final ok = hasMain && hasBugreportTxt;
    return BugreportValidation(
      ok: ok,
      messages: messages,
      sizeBytes: bytes.length,
      captureHint: captureHint,
    );
  } catch (e) {
    return BugreportValidation(
      ok: false,
      messages: ['ZIP parse failed: $e'],
      sizeBytes: bytes.length,
    );
  }
}
