import 'dart:typed_data';

Future<Uint8List> readPathBytes(String path) {
  throw UnsupportedError('Local file paths are not available on this platform: $path');
}
