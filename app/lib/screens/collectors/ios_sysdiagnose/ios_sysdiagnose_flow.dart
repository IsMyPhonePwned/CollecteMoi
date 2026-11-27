import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../../config/tenant_config.dart';
import '../../../l10n/app_localizations.dart';
import '../../../models/models.dart';
import '../../../services/file_bytes.dart';
import '../../../services/platform_bridge.dart';
import '../../../services/sysdiagnose_validator.dart';
import '../../../services/upload_queue.dart';
import '../../../widgets/guide_scaffold.dart';

class IosSysdiagnoseFlow extends StatefulWidget {
  const IosSysdiagnoseFlow({super.key});

  @override
  State<IosSysdiagnoseFlow> createState() => _IosSysdiagnoseFlowState();
}

class _IosSysdiagnoseFlowState extends State<IosSysdiagnoseFlow> {
  final _bridge = PlatformBridge();
  int _step = 1;
  String? _receivedPath;
  Uint8List? _pickedBytes;

  static const total = 6;

  @override
  void initState() {
    super.initState();
    _pullPending();
    _bridge.shareEvents.listen((event) {
      final path = event['path']?.toString();
      if (path != null && mounted) {
        setState(() {
          _receivedPath = path;
          _step = 6;
        });
      }
    });
  }

  Future<void> _pullPending() async {
    final jobs = await _bridge.pendingShareJobs();
    if (jobs.isEmpty || !mounted) return;
    final path = jobs.first['path']?.toString();
    if (path != null) {
      setState(() {
        _receivedPath = path;
        _step = 6;
      });
    }
  }

  Future<void> _importFallback() async {
    final file = await FilePicker.pickFile(type: FileType.any);
    if (file == null) return;
    final bytes = await file.readAsBytes();
    setState(() {
      _receivedPath = file.name;
      _pickedBytes = bytes;
      _step = 6;
    });
  }

  Future<void> _continue() async {
    if (_step < 6) {
      setState(() => _step += 1);
      return;
    }
    if (_receivedPath == null) {
      await _importFallback();
      return;
    }
    final bytes = _pickedBytes ?? await readPathBytes(_receivedPath!);
    if (!mounted) return;
    final check = validateSysdiagnose(bytes: bytes, fileName: _receivedPath);
    if (!check.ok) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            context.tr('This does not look like a sysdiagnose:\n{details}')
                .replaceAll('{details}', check.messages.join('\n')),
          ),
        ),
      );
      return;
    }
    final job = await context.read<UploadQueue>().enqueueBytes(
          artefact: ArtefactKind.iosSysdiagnose,
          bytes: bytes,
          label: _receivedPath!,
        );
    if (!mounted) return;
    context.push('/consent/${job.id}');
  }

  @override
  Widget build(BuildContext context) {
    context.watch<TenantConfig>();
    final guide = GuideCopy(
      title: context.branded((c) => c.iosSysdiagnose.title),
      headline: context.branded((c) => c.iosSysdiagnose.headline),
      body: context.branded((c) => c.iosSysdiagnose.body),
    );
    return GuideScaffold(
      title: guide.title,
      step: _step,
      totalSteps: total,
      primaryLabel: _step == 6
          ? (_receivedPath == null ? context.tr('Import from Files') : context.tr('Continue to consent'))
          : context.tr('Next'),
      onPrimary: _continue,
      secondaryLabel: _step >= 5 ? context.tr('Import from Files instead') : null,
      onSecondary: _importFallback,
      child: switch (_step) {
        1 => GuideBody(
            headline: guide.headline,
            body: guide.body,
          ),
        2 => GuideBody(
            headline: context.tr('Trigger sysdiagnose'),
            body: context.tr('Press and hold both volume buttons and the side button together for about 1 to 1.5 seconds, then release on the short vibration. A screenshot flash means the hold was too short or too long. Older Home-button models use a different combo.'),
            illustration: _HintCard(
              icon: Icons.vibration,
              text: context.tr('Volume + + Volume − + Side · ~1.5s'),
            ),
          ),
        3 => GuideBody(
            headline: context.tr('Wait about 10 minutes'),
            body: context.tr('We cannot see progress — iOS does not expose it. Leave the phone unlocked if you can. We will remind you with a local notification when possible.'),
          ),
        4 => GuideBody(
            headline: context.tr('Find the archive'),
            body: context.tr('Settings → Privacy & Security → Analytics & Improvements → Analytics Data. Open the newest file named sysdiagnose_YYYY.MM.DD_….tar.gz.'),
          ),
        5 => GuideBody(
            headline: context.tr('Share into CollecteMoi'),
            body: context.tr('Tap the file, then Share, then CollecteMoi. The share extension copies the archive into the app without loading it into memory.'),
          ),
        _ => GuideBody(
            headline: _receivedPath == null
                ? context.tr('No archive yet')
                : context.tr('Sysdiagnose received'),
            body: _receivedPath == null
                ? context.tr('Share from Analytics Data, or use Import from Files if you saved it there.')
                : context.tr('Ready:\n{path}').replaceAll('{path}', _receivedPath!),
          ),
      },
    );
  }
}

class _HintCard extends StatelessWidget {
  const _HintCard({required this.icon, required this.text});
  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(20),
      decoration: BoxDecoration(
        color: Theme.of(context).colorScheme.primaryContainer.withValues(alpha: 0.45),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Row(
        children: [
          Icon(icon, size: 36),
          const SizedBox(width: 16),
          Expanded(child: Text(text, style: Theme.of(context).textTheme.titleMedium)),
        ],
      ),
    );
  }
}
