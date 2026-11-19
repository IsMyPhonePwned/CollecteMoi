import 'package:file_picker/file_picker.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../../models/models.dart';
import '../../services/bugreport_validator.dart';
import '../../services/sysdiagnose_validator.dart';
import '../../services/upload_queue.dart';
import '../../widgets/theme_picker_button.dart';

/// Browser / desktop lab helper: pick any file and submit it as an artefact.
class LabUploadScreen extends StatefulWidget {
  const LabUploadScreen({super.key});

  @override
  State<LabUploadScreen> createState() => _LabUploadScreenState();
}

class _LabUploadScreenState extends State<LabUploadScreen> {
  ArtefactKind _artefact = ArtefactKind.androidBugreport;
  String? _fileName;
  int? _size;
  Uint8List? _bytes;
  String? _error;

  Future<void> _pick() async {
    setState(() => _error = null);
    final file = await FilePicker.pickFile();
    if (file == null) return;
    try {
      final bytes = await file.readAsBytes();
      setState(() {
        _fileName = file.name;
        _bytes = bytes;
        _size = bytes.length;
      });
    } catch (e) {
      setState(() => _error = e.toString());
    }
  }

  Future<void> _continue() async {
    final bytes = _bytes;
    if (bytes == null) return;

    if (_artefact == ArtefactKind.androidBugreport) {
      final v = validateAndroidBugreport(bytes);
      if (!v.ok) {
        setState(() => _error = v.messages.join('\n'));
        return;
      }
    } else if (_artefact == ArtefactKind.iosSysdiagnose) {
      final v = validateSysdiagnose(bytes: bytes, fileName: _fileName);
      if (!v.ok) {
        setState(() => _error = v.messages.join('\n'));
        return;
      }
    }

    final job = await context.read<UploadQueue>().enqueueBytes(
      artefact: _artefact,
      bytes: bytes,
      label: _fileName ?? 'upload.bin',
    );
    if (!mounted) return;
    context.push('/consent/${job.id}');
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(
        title: Text(context.tr('Lab file upload')),
        actions: const [ThemePickerButton()],
      ),
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 560),
          child: ListView(
            padding: const EdgeInsets.all(24),
            children: [
              Text(
                context.tr(kIsWeb ? 'Web lab client' : 'Lab file client'),
                style: theme.textTheme.headlineSmall,
              ),
              const SizedBox(height: 8),
              Text(
                context.tr('Enroll with token demo, then pick a file (bugreport ZIP, sysdiagnose archive, or any test blob) and send it through the same consent + chunked upload path as the phone app.'),
                style: theme.textTheme.bodyLarge?.copyWith(height: 1.4),
              ),
              const SizedBox(height: 24),
              DropdownButtonFormField<ArtefactKind>(
                // ignore: deprecated_member_use
                value: _artefact,
                decoration: InputDecoration(
                  labelText: context.tr('Artefact kind'),
                  border: OutlineInputBorder(),
                ),
                items: ArtefactKind.values
                    .map(
                      (k) => DropdownMenuItem(value: k, child: Text(context.tr(k.label))),
                    )
                    .toList(),
                onChanged: (v) => setState(() => _artefact = v ?? _artefact),
              ),
              const SizedBox(height: 16),
              OutlinedButton.icon(
                onPressed: _pick,
                icon: const Icon(Icons.upload_file),
                label: Text(
                  context.tr(_fileName == null ? 'Choose file' : 'Choose another file'),
                ),
              ),
              if (_fileName != null) ...[
                const SizedBox(height: 12),
                Card(
                  child: ListTile(
                    leading: const Icon(Icons.insert_drive_file_outlined),
                    title: Text(_fileName!),
                    subtitle: Text(
                      _size == null
                          ? ''
                          : '${(_size! / (1024 * 1024)).toStringAsFixed(2)} MB',
                    ),
                  ),
                ),
              ],
              if (_error != null) ...[
                const SizedBox(height: 12),
                Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
              ],
              const SizedBox(height: 24),
              FilledButton(
                onPressed: _bytes == null ? null : _continue,
                child: Text(context.tr('Continue to consent')),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
