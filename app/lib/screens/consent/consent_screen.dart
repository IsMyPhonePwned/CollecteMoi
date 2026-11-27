import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../config/tenant_config.dart';
import '../../l10n/app_localizations.dart';
import '../../models/models.dart';
import '../../services/upload_queue.dart';
import '../../util/format_bytes.dart';
import '../../widgets/settings_button.dart';

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

  String _disclosure(BuildContext context, ArtefactKind kind) => switch (kind) {
        ArtefactKind.androidBugreport =>
          context.branded((c) => c.consent.disclosureAndroid),
        ArtefactKind.iosSysdiagnose =>
          context.branded((c) => c.consent.disclosureIos),
        ArtefactKind.messageReport =>
          context.branded((c) => c.consent.disclosureMessage),
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

  void _send(UploadJob job) {
    if (_busy) return;
    setState(() => _busy = true);
    // Start encrypt/upload in the background (Android foreground service keeps
    // it alive if the user leaves the app), then return to My data immediately.
    context.read<UploadQueue>().startConsentedUpload(job.id);
    if (!mounted) return;
    final messenger = ScaffoldMessenger.of(context);
    final startedMsg = context.tr(
      'Upload started in the background. You can leave this screen — progress is on My data.',
    );
    context.go('/my-data');
    WidgetsBinding.instance.addPostFrameCallback((_) {
      messenger.showSnackBar(
        SnackBar(content: Text(startedMsg), duration: const Duration(seconds: 4)),
      );
    });
  }

  @override
  Widget build(BuildContext context) {
    final queue = context.watch<UploadQueue>();
    context.watch<TenantConfig>();
    final copyTitle = context.branded((c) => c.consent.title);
    final copyFooter = context.branded((c) => c.consent.footer);
    final copyCheckbox = context.branded((c) => c.consent.checkbox);
    final copyButton = context.branded((c) => c.consent.button);
    UploadJob? job;
    for (final j in queue.jobs) {
      if (j.id == widget.jobId) {
        job = j;
        break;
      }
    }
    if (job == null) {
      return Scaffold(
        appBar: AppBar(
          title: Text(copyTitle),
          actions: const [SettingsButton()],
        ),
        body: Center(child: Text(context.tr('Job not found'))),
      );
    }
    final activeJob = job;
    final sizeLabel = activeJob.sizeBytes == null
        ? context.tr('unknown size')
        : formatBytes(activeJob.sizeBytes);
    final payloadMissing = _payloadReady == false;

    return Scaffold(
      appBar: AppBar(
        title: Text(copyTitle),
        actions: const [SettingsButton()],
      ),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                context.tr(activeJob.artefact.label),
                style: Theme.of(context).textTheme.headlineSmall,
              ),
              const SizedBox(height: 8),
              Text(
                context.tr('Local file · {size}')
                    .replaceAll('{size}', sizeLabel),
              ),
              Text(
                activeJob.localPath,
                style: Theme.of(context).textTheme.bodySmall,
              ),
              if (payloadMissing) ...[
                const SizedBox(height: 12),
                Text(
                  context.tr(
                    'The captured file is no longer on this phone. Go back and share or import it again.',
                  ),
                  style: TextStyle(
                    color: Theme.of(context).colorScheme.error,
                  ),
                ),
              ] else if (_payloadReady == true) ...[
                const SizedBox(height: 8),
                Text(
                  context.tr(
                    'Ready to encrypt — file kept on this phone until upload finishes.',
                  ),
                  style: Theme.of(context).textTheme.bodySmall,
                ),
              ],
              const SizedBox(height: 20),
              Text(
                _disclosure(context, activeJob.artefact),
                style: Theme.of(context)
                    .textTheme
                    .bodyLarge
                    ?.copyWith(height: 1.4),
              ),
              const SizedBox(height: 12),
              Text(
                copyFooter,
                style: Theme.of(context).textTheme.bodyMedium,
              ),
              const Spacer(),
              const SizedBox(height: 16),
              CheckboxListTile(
                value: _accepted,
                onChanged: payloadMissing || _busy
                    ? null
                    : (v) => setState(() => _accepted = v ?? false),
                title: Text(copyCheckbox),
                controlAffinity: ListTileControlAffinity.leading,
                contentPadding: EdgeInsets.zero,
              ),
              const SizedBox(height: 8),
              FilledButton(
                onPressed: !_accepted || payloadMissing || _busy
                    ? null
                    : () => _send(activeJob),
                child: Text(copyButton),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
