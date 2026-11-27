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

String _basename(String path) {
  final normalized = path.replaceAll('\\', '/');
  final parts = normalized.split('/');
  return parts.isEmpty ? normalized : parts.last;
}

String _normName(String path) {
  var n = path.replaceAll('\\', '/');
  while (n.startsWith('./')) {
    n = n.substring(2);
  }
  return n;
}

bool _looksLikeBugreportName(String name) {
  final base = _basename(name).toLowerCase();
  if (base.startsWith('bugreport') &&
      (base.endsWith('.txt') ||
          base.endsWith('.txt.gz') ||
          base.endsWith('.zip') ||
          base.endsWith('.gz'))) {
    return true;
  }
  if (base.startsWith('dumpstate') &&
      (base.endsWith('.txt') ||
          base.endsWith('.txt.gz') ||
          base.endsWith('.gz') ||
          base.endsWith('.log'))) {
    return true;
  }
  return false;
}

bool _looksLikeDumpstateText(String head) {
  final h = head.toLowerCase();
  return h.contains('== dumpstate:') ||
      h.contains('dumpstate:') ||
      h.contains('bugreport') ||
      h.contains('build fingerprint:') ||
      h.contains('----- beginning of');
}

Uint8List _fileBytes(ArchiveFile f) {
  final bytes = f.readBytes();
  if (bytes != null && bytes.isNotEmpty) return bytes;
  final c = f.content;
  if (c.isNotEmpty) return c;
  return Uint8List(0);
}

String? _decodeHead(List<int> bytes, {int max = 400}) {
  if (bytes.isEmpty) return null;
  final take = bytes.length < max ? bytes.length : max;
  return utf8.decode(bytes.sublist(0, take), allowMalformed: true);
}

bool _namesContain(List<String> names, String named) {
  final want = _basename(named);
  return names.any((n) {
    final base = _basename(n);
    return base == want || base == named || n.endsWith('/$want') || n.endsWith(named);
  });
}

/// Client-side checks before consent (architecture § Android validation).
///
/// Accepts classic AOSP ZIPs (`main_entry.txt` + `bugreport-*.txt`), modern
/// OEM variants (`dumpstate*.txt`, nested paths / nested bugreport ZIPs), and
/// plain dumpstate text / gzip for lab fallbacks.
BugreportValidation validateAndroidBugreport(Uint8List bytes) {
  final messages = <String>[];
  if (bytes.length < 4) {
    return BugreportValidation(ok: false, messages: ['File too small']);
  }

  // Gzip of dumpstate / bugreport text (some OEM share targets).
  if (bytes[0] == 0x1f && bytes[1] == 0x8b) {
    try {
      final raw = const GZipDecoder().decodeBytes(bytes);
      final head = _decodeHead(raw) ?? '';
      if (_looksLikeDumpstateText(head)) {
        return BugreportValidation(
          ok: true,
          messages: ['Gzip dumpstate / bugreport text — accepted'],
          sizeBytes: bytes.length,
          captureHint: head
              .split('\n')
              .map((l) => l.trim())
              .firstWhere((l) => l.isNotEmpty, orElse: () => ''),
        );
      }
    } catch (e) {
      messages.add('Gzip parse failed: $e');
    }
  }

  final isZip = bytes[0] == 0x50 && bytes[1] == 0x4b;
  if (!isZip) {
    final head = utf8.decode(bytes.take(400).toList(), allowMalformed: true);
    if (_looksLikeDumpstateText(head)) {
      messages.add('Plain dumpstate text (not a ZIP) — accepted');
      return BugreportValidation(
        ok: true,
        messages: messages,
        sizeBytes: bytes.length,
        captureHint: head
            .split('\n')
            .map((l) => l.trim())
            .firstWhere((l) => l.isNotEmpty, orElse: () => ''),
      );
    }
    return BugreportValidation(
      ok: false,
      messages: [
        ...messages,
        'Not a ZIP / gzip and not a dumpstate text header',
      ],
      sizeBytes: bytes.length,
    );
  }

  try {
    return _validateZip(bytes, messages, depth: 0);
  } catch (e) {
    return BugreportValidation(
      ok: false,
      messages: [...messages, 'ZIP parse failed: $e'],
      sizeBytes: bytes.length,
    );
  }
}

BugreportValidation _validateZip(
  Uint8List bytes,
  List<String> messages, {
  required int depth,
}) {
  final archive = ZipDecoder().decodeBytes(bytes, verify: false);
  final names = archive
      .map((f) => _normName(f.name))
      .where((n) => n.isNotEmpty && !n.endsWith('/'))
      .toList();
  final bases = names.map(_basename).toList();

  final hasMain = names.any(
    (n) => _basename(n).toLowerCase() == 'main_entry.txt',
  );
  if (hasMain) {
    messages.add('Found main_entry.txt');
  } else {
    messages.add('ZIP missing main_entry.txt');
  }

  final bugreportEntries = names.where(_looksLikeBugreportName).toList();
  final hasBugreportTxt = bugreportEntries.any((n) {
    final b = _basename(n).toLowerCase();
    return b.endsWith('.txt') || b.endsWith('.txt.gz') || b.endsWith('.gz');
  });
  final hasNestedBugreportZip = bugreportEntries.any(
    (n) => _basename(n).toLowerCase().endsWith('.zip'),
  );
  if (hasBugreportTxt) {
    messages.add('Found bugreport / dumpstate text entry');
  } else if (hasNestedBugreportZip) {
    messages.add('Found nested bugreport ZIP');
  } else {
    messages.add('No bugreport-*.txt entry found');
  }

  final hasDumpstateBoard = bases.any(
    (b) => b.toLowerCase().startsWith('dumpstate'),
  );
  final hasVersion = bases.any((b) => b.toLowerCase() == 'version.txt');
  final hasFsOrProto = names.any((n) {
    final lower = n.toLowerCase();
    return lower.startsWith('fs/') ||
        lower.startsWith('proto/') ||
        lower.contains('/fs/') ||
        lower.contains('/proto/');
  });
  if (hasDumpstateBoard) messages.add('Found dumpstate* entry');
  if (hasVersion) messages.add('Found version.txt');
  if (hasFsOrProto) messages.add('Found FS/ or proto/ tree');

  String? captureHint;
  var mainNamesPresent = true;
  ArchiveFile? mainFile;
  for (final f in archive) {
    if (_basename(f.name).toLowerCase() == 'main_entry.txt') {
      mainFile = f;
      break;
    }
  }
  if (mainFile != null) {
    final content = utf8
        .decode(_fileBytes(mainFile), allowMalformed: true)
        .trim()
        .split(RegExp(r'[\r\n]+'))
        .map((l) => l.trim())
        .where((l) => l.isNotEmpty)
        .toList();
    if (content.isNotEmpty) {
      captureHint = content.first;
      for (final named in content) {
        if (!_namesContain(names, named)) {
          mainNamesPresent = false;
          messages.add('main_entry names "$named" but that file is absent');
        }
      }
    }
  }

  // Peek named dumpstate/bugreport text for headers (OEM renames without prefix).
  var hasDumpstateContent = false;
  for (final f in archive) {
    final base = _basename(f.name).toLowerCase();
    if (f.isDirectory || !(base.endsWith('.txt') || base.endsWith('.txt.gz'))) {
      continue;
    }
    if (f.size > 2 * 1024 * 1024) continue;
    final interesting = base.contains('dumpstate') ||
        base.contains('bugreport') ||
        base == 'main_entry.txt' ||
        (!hasBugreportTxt && base.endsWith('.txt'));
    if (!interesting) continue;
    final head = _decodeHead(_fileBytes(f));
    if (head != null && _looksLikeDumpstateText(head)) {
      hasDumpstateContent = true;
      captureHint ??= head
          .split('\n')
          .map((l) => l.trim())
          .firstWhere((l) => l.isNotEmpty, orElse: () => '');
      messages.add('Dumpstate header detected in ${_basename(f.name)}');
      break;
    }
  }

  // Unwrap one nested bugreport ZIP (Android share sometimes wraps again).
  if (!hasBugreportTxt && hasNestedBugreportZip && depth < 1) {
    for (final f in archive) {
      final base = _basename(f.name).toLowerCase();
      if (!base.endsWith('.zip')) continue;
      if (!base.startsWith('bugreport') && !_looksLikeBugreportName(f.name)) {
        continue;
      }
      final inner = _fileBytes(f);
      if (inner.length < 4 || inner[0] != 0x50 || inner[1] != 0x4b) continue;
      messages.add('Inspecting nested ${_basename(f.name)}');
      final nested = _validateZip(inner, messages, depth: depth + 1);
      if (nested.ok) {
        return BugreportValidation(
          ok: true,
          messages: messages,
          sizeBytes: bytes.length,
          captureHint: nested.captureHint ?? captureHint,
        );
      }
    }
  }

  // Classic: main_entry + bugreport text. Also accept modern layouts with
  // dumpstate content / board dumps even when the classic bugreport-*.txt name
  // is missing (OEM variants / nested paths).
  final ok = (hasMain && hasBugreportTxt && mainNamesPresent) ||
      (hasMain && (hasBugreportTxt || hasDumpstateBoard || hasDumpstateContent)) ||
      (hasBugreportTxt && (hasDumpstateContent || hasVersion || hasFsOrProto || hasMain)) ||
      (hasDumpstateContent && (hasVersion || hasFsOrProto || hasDumpstateBoard));

  if (!ok) {
    messages.add('ZIP did not match known Android bugreport layouts');
  }

  return BugreportValidation(
    ok: ok,
    messages: messages,
    sizeBytes: bytes.length,
    captureHint: captureHint,
  );
}
