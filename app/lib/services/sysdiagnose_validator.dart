import 'dart:typed_data';

class SysdiagnoseValidation {
  SysdiagnoseValidation({
    required this.ok,
    required this.messages,
    this.sizeBytes,
  });

  final bool ok;
  final List<String> messages;
  final int? sizeBytes;
}

/// Lightweight checks (name + gzip magic). Full tar listing is server-side.
SysdiagnoseValidation validateSysdiagnose({
  required Uint8List bytes,
  String? fileName,
}) {
  final messages = <String>[];
  final name = fileName ?? '';
  final nameOk = name.isEmpty ||
      RegExp(r'sysdiagnose_.*\.(tar\.gz|tgz|tar)$', caseSensitive: false)
          .hasMatch(name);
  if (!nameOk) {
    messages.add('Name does not look like sysdiagnose_*.tar.gz (continuing anyway if gzip)');
  } else if (name.isNotEmpty) {
    messages.add('Filename matches sysdiagnose pattern');
  }

  final gzip = bytes.length >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b;
  if (!gzip) {
    messages.add('Missing gzip magic — not a .tar.gz');
    return SysdiagnoseValidation(
      ok: false,
      messages: messages,
      sizeBytes: bytes.length,
    );
  }
  messages.add('Valid gzip header');

  return SysdiagnoseValidation(
    ok: true,
    messages: messages,
    sizeBytes: bytes.length,
  );
}
