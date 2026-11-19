import 'dart:io';
import 'dart:typed_data';

import 'package:path_provider/path_provider.dart';

/// Persists plaintext payloads under `<documents>/payloads/<jobId>.bin`
/// so consent/upload survives process death during long captures.
class PayloadStore {
  Future<Directory> _dir() async {
    final root = await getApplicationDocumentsDirectory();
    final dir = Directory('${root.path}/payloads');
    if (!await dir.exists()) {
      await dir.create(recursive: true);
    }
    return dir;
  }

  Future<File> _fileFor(String jobId) async {
    final dir = await _dir();
    return File('${dir.path}/$jobId.bin');
  }

  Future<String?> write(String jobId, Uint8List bytes) async {
    final file = await _fileFor(jobId);
    await file.writeAsBytes(bytes, flush: true);
    return file.path;
  }

  Future<Uint8List?> read(String? path, {String? jobId}) async {
    File? file;
    if (path != null && path.isNotEmpty) {
      file = File(path);
    } else if (jobId != null) {
      file = await _fileFor(jobId);
    }
    if (file == null || !await file.exists()) return null;
    return file.readAsBytes();
  }

  Future<void> delete(String? path, {String? jobId}) async {
    try {
      if (path != null && path.isNotEmpty) {
        final f = File(path);
        if (await f.exists()) await f.delete();
      }
      if (jobId != null) {
        final f = await _fileFor(jobId);
        if (await f.exists()) await f.delete();
      }
    } catch (_) {
      // Best-effort cleanup.
    }
  }

  Future<bool> exists(String? path, {String? jobId}) async {
    if (path != null && path.isNotEmpty) {
      return File(path).exists();
    }
    if (jobId != null) {
      return (await _fileFor(jobId)).exists();
    }
    return false;
  }
}
