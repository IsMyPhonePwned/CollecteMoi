import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../config/tenant_config.dart';
import '../../l10n/app_localizations.dart';
import '../../models/models.dart';
import '../../services/upload_queue.dart';
import '../../widgets/theme_picker_button.dart';

class ConsentScreen extends StatefulWidget {
  const ConsentScreen({super.key, required this.jobId});

  final String jobId;

  @override
  State<ConsentScreen> createState() => _ConsentScreenState();
}

class _ConsentScreenState extends State<ConsentScreen> {
  bool _accepted = false;
  bool _busy = false;
  bool? _payloadReady;

  String _disclosure(ArtefactKind kind, ConsentCopy copy) => switch (kind) {
    ArtefactKind.androidBugreport => copy.disclosureAndroid,
    ArtefactKind.iosSysdiagnose => copy.disclosureIos,
    ArtefactKind.messageReport => copy.disclosureMessage,
  };

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _checkPayload());
  }

  Future<void> _checkPayload() async {
    final ready = await context.read<UploadQueue>().hasPayload(widget.jobId);
    if (!mounted) return;
    setState(() => _payloadReady = ready);
  }

  Future<void> _send(UploadJob job) async {
    setState(() => _busy = true);
    await context.read<UploadQueue>().consentAndUpload(job.id);
    if (!mounted) return;
    setState(() => _busy = false);
    final updated = context.read<UploadQueue>().jobs.firstWhere(
      (j) => j.id == job.id,
    );
    if (updated.status == UploadJobStatus.complete) {
      context.go('/my-data');
    } else {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text(updated.error ?? context.tr('Upload failed'))));
      await _checkPayload();
    }
  }

  @override
  Widget build(BuildContext context) {
    final queue = context.watch<UploadQueue>();
    UploadJob? job;
    for (final j in queue.jobs) {
      if (j.id == widget.jobId) {
        job = j;
        break;
      }
    }

    final copy = context.watch<TenantConfig>().copy.consent;

    if (job == null) {
      return Scaffold(
        appBar: AppBar(
          title: Text(context.tr(copy.title)),
          actions: const [ThemePickerButton()],
        ),
        body: Center(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(context.tr('Upload job not found')),
                const SizedBox(height: 16),
                FilledButton(
                  onPressed: () => context.go('/my-data'),
                  child: Text(context.tr('Back to My data')),
                ),
              ],
            ),
          ),
        ),
      );
    }

    final sizeLabel = job.sizeBytes == null
        ? context.tr('unknown size')
        : '${(job.sizeBytes! / (1024 * 1024)).toStringAsFixed(1)} MB';
    final payloadMissing = _payloadReady == false;

    return Scaffold(
      appBar: AppBar(
        title: Text(context.tr(copy.title)),
        actions: const [ThemePickerButton()],
      ),
      body: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              context.tr(job.artefact.label),
              style: Theme.of(context).textTheme.headlineSmall,
            ),
            const SizedBox(height: 8),
            Text(context.tr('Local file · {size}').replaceAll('{size}', sizeLabel)),
            Text(job.localPath, style: Theme.of(context).textTheme.bodySmall),
            if (payloadMissing) ...[
              const SizedBox(height: 12),
              Text(
                context.tr('The captured file is no longer on this phone. Go back and share or import it again.'),
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ] else if (_payloadReady == true) ...[
              const SizedBox(height: 8),
              Text(
                context.tr('Ready to encrypt — file kept on this phone until upload finishes.'),
                style: Theme.of(context).textTheme.bodySmall,
              ),
            ],
            const SizedBox(height: 20),
            Text(
              context.tr(_disclosure(job.artefact, copy)),
              style: Theme.of(
                context,
              ).textTheme.bodyLarge?.copyWith(height: 1.4),
            ),
            const SizedBox(height: 12),
            Text(context.tr(copy.footer), style: Theme.of(context).textTheme.bodyMedium),
            const Spacer(),
            CheckboxListTile(
              contentPadding: EdgeInsets.zero,
              value: _accepted,
              onChanged: _busy || payloadMissing
                  ? null
                  : (v) => setState(() => _accepted = v ?? false),
              title: Text(context.tr(copy.checkbox)),
            ),
            FilledButton(
              onPressed: !_accepted || _busy || payloadMissing
                  ? null
                  : () => _send(job!),
              child: _busy
                  ? const SizedBox(
                      height: 22,
                      width: 22,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : Text(context.tr(copy.button)),
            ),
            if (payloadMissing)
              TextButton(
                onPressed: () => context.go('/'),
                child: Text(context.tr('Back to home')),
              ),
            if (job.status == UploadJobStatus.uploading ||
                job.status == UploadJobStatus.encrypting)
              Padding(
                padding: const EdgeInsets.only(top: 12),
                child: LinearProgressIndicator(
                  value: job.progress == 0 ? null : job.progress,
                ),
              ),
          ],
        ),
      ),
    );
  }
}
