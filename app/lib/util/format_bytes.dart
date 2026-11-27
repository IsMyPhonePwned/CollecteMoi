/// Human-readable byte size (B / KB / MB / GB).
String formatBytes(int? bytes) {
  if (bytes == null) return '—';
  if (bytes < 1024) return '$bytes B';
  if (bytes < 1024 * 1024) {
    final kb = bytes / 1024;
    return kb >= 10
        ? '${kb.toStringAsFixed(0)} KB'
        : '${kb.toStringAsFixed(1)} KB';
  }
  if (bytes < 1024 * 1024 * 1024) {
    final mb = bytes / (1024 * 1024);
    return mb >= 10
        ? '${mb.toStringAsFixed(0)} MB'
        : '${mb.toStringAsFixed(1)} MB';
  }
  final gb = bytes / (1024 * 1024 * 1024);
  return '${gb.toStringAsFixed(2)} GB';
}
