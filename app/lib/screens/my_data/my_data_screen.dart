import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../config/tenant_config.dart';
import '../../l10n/app_localizations.dart';
import '../../models/models.dart';
import '../../services/api_client.dart';
import '../../services/upload_queue.dart';
import '../../widgets/theme_picker_button.dart';

class MyDataScreen extends StatelessWidget {
  const MyDataScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final queue = context.watch<UploadQueue>();
    final copy = context.watch<TenantConfig>().copy.myData;
    final session = queue.session;
    final theme = Theme.of(context);

    return Scaffold(
      appBar: AppBar(
        title: Text(context.tr(copy.title)),
        actions: const [ThemePickerButton()],
      ),
      body: ListView(
        padding: const EdgeInsets.all(20),
        children: [
          if (session != null) ...[
            Text(context.tr(copy.deviceHeading), style: theme.textTheme.titleMedium),
            Text('${session.displayName} · ${session.deviceId}'),
            Text('${context.tr('Case')} ${session.caseId}', style: theme.textTheme.bodySmall),
            const SizedBox(height: 20),
            _ServerHistory(session: session),
            const SizedBox(height: 20),
          ],
          Text(context.tr('On this device'), style: theme.textTheme.titleLarge),
          const SizedBox(height: 4),
          Text(context.tr(copy.artefactsHeading), style: theme.textTheme.bodyMedium),
          const SizedBox(height: 8),
          if (queue.jobs.isEmpty) Text(context.tr(copy.empty)),
          ...queue.jobs.map((job) => _JobCard(job: job)),
          const SizedBox(height: 24),
          OutlinedButton(
            onPressed: session == null
                ? null
                : () async {
                    final ok = await showDialog<bool>(
                      context: context,
                      builder: (ctx) => AlertDialog(
                        title: Text('${context.tr(copy.clearButton)}?'),
                        content: Text(context.tr('Removes the device session from this phone. Upload history stays until you discard each job. This does not erase data already sent to the lab.')),
                        actions: [
                          TextButton(
                            onPressed: () => Navigator.pop(ctx, false),
                            child: Text(context.tr('Cancel')),
                          ),
                          FilledButton(
                            onPressed: () => Navigator.pop(ctx, true),
                            child: Text(context.tr('Clear')),
                          ),
                        ],
                      ),
                    );
                    if (ok != true || !context.mounted) return;
                    await context.read<UploadQueue>().clearSession();
                    if (context.mounted) {
                      ScaffoldMessenger.of(context).showSnackBar(
                        SnackBar(
                          content: Text(context.tr('Local enrollment cleared.')),
                        ),
                      );
                      context.go('/enroll');
                    }
                  },
            child: Text(context.tr(copy.clearButton)),
          ),
        ],
      ),
    );
  }
}

class _ServerHistory extends StatefulWidget {
  const _ServerHistory({required this.session});

  final EnrollmentSession session;

  @override
  State<_ServerHistory> createState() => _ServerHistoryState();
}

class _ServerHistoryState extends State<_ServerHistory> {
  List<ServerSubmission> _submissions = [];
  bool _loading = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final submissions = await context.read<ApiClient>().clientSubmissions(
        deviceId: widget.session.deviceId,
        noiseKeyHex: widget.session.noiseKeyHex,
      );
      if (!mounted) return;
      setState(() => _submissions = submissions);
    } catch (_) {
      if (!mounted) return;
      setState(() => _error = 'Submission history could not be loaded.');
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                context.tr('Submission history'),
                style: theme.textTheme.titleLarge,
              ),
            ),
            IconButton(
              tooltip: context.tr('Refresh submission history'),
              onPressed: _loading ? null : _load,
              icon: const Icon(Icons.refresh),
            ),
          ],
        ),
        if (_loading)
          const Padding(
            padding: EdgeInsets.symmetric(vertical: 20),
            child: Center(child: CircularProgressIndicator()),
          )
        else if (_error != null)
          Card(
            child: ListTile(
              leading: Icon(
                Icons.cloud_off_outlined,
                color: theme.colorScheme.error,
              ),
              title: Text(context.tr(_error!)),
              trailing: TextButton(
                onPressed: _load,
                child: Text(context.tr('Retry')),
              ),
            ),
          )
        else if (_submissions.isEmpty)
          Card(
            child: ListTile(
              leading: const Icon(Icons.inbox_outlined),
              title: Text(context.tr('No submissions received yet')),
              subtitle: Text(
                context.tr('Completed uploads will appear here after the lab receives them.'),
              ),
            ),
          )
        else
          ..._submissions.map((submission) {
            final status = _submissionStatus(context, submission.status);
            final size = submission.sizeBytes < 1024 * 1024
                ? '${(submission.sizeBytes / 1024).toStringAsFixed(0)} KB'
                : '${(submission.sizeBytes / (1024 * 1024)).toStringAsFixed(1)} MB';
            return Card(
              child: ListTile(
                leading: Icon(
                  submission.status == 'failed'
                      ? Icons.error_outline
                      : submission.status == 'done'
                      ? Icons.check_circle_outline
                      : Icons.cloud_upload_outlined,
                  color: submission.status == 'failed'
                      ? theme.colorScheme.error
                      : submission.status == 'done'
                      ? theme.colorScheme.primary
                      : theme.colorScheme.onSurfaceVariant,
                ),
                title: Text(context.tr(submission.artefact.label)),
                subtitle: Text(
                  '${_localizedDateTime(context, submission.submittedAt)} · '
                  '$size · ${submission.uploadId.substring(0, 8)}',
                ),
                trailing: Chip(
                  label: Text(status),
                  visualDensity: VisualDensity.compact,
                ),
              ),
            );
          }),
      ],
    );
  }

  String _submissionStatus(BuildContext context, String status) => switch (status) {
    'done' => context.tr('Received'),
    'failed' => context.tr('Processing failed'),
    'claimed' || 'pending' => context.tr('Processing'),
    _ => context.tr('Submitted'),
  };
}

String _localizedDateTime(BuildContext context, DateTime value) {
  final localizations = MaterialLocalizations.of(context);
  final local = value.toLocal();
  return '${localizations.formatMediumDate(local)} ${localizations.formatTimeOfDay(TimeOfDay.fromDateTime(local))}';
}

class _JobCard extends StatelessWidget {
  const _JobCard({required this.job});

  final UploadJob job;

  @override
  Widget build(BuildContext context) {
    final queue = context.read<UploadQueue>();
    final theme = Theme.of(context);
    final size = job.sizeBytes == null
        ? null
        : '${(job.sizeBytes! / (1024 * 1024)).toStringAsFixed(1)} MB';

    return Card(
      margin: const EdgeInsets.only(bottom: 10),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 10, 8, 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ListTile(
              contentPadding: EdgeInsets.zero,
              leading: Icon(_iconFor(job.status)),
              title: Text(context.tr(job.artefact.label)),
              subtitle: Text(
                [_statusLabel(context, job.status), ?size, ?job.error].join(' · '),
                maxLines: 3,
                overflow: TextOverflow.ellipsis,
              ),
              trailing:
                  job.status == UploadJobStatus.uploading ||
                      job.status == UploadJobStatus.encrypting
                  ? SizedBox(
                      width: 36,
                      height: 36,
                      child: CircularProgressIndicator(
                        value: job.progress == 0 ? null : job.progress,
                      ),
                    )
                  : null,
            ),
            if (job.canResumeConsent && !queue.busy)
              Padding(
                padding: const EdgeInsets.only(left: 8, right: 8, bottom: 4),
                child: Row(
                  children: [
                    FilledButton.tonal(
                      onPressed: () async {
                        await queue.prepareRetry(job.id);
                        if (!context.mounted) return;
                        if (!await queue.hasPayload(job.id)) {
                          if (!context.mounted) return;
                          ScaffoldMessenger.of(context).showSnackBar(
                            SnackBar(
                              content: Text(
                                job.error ??
                                    context.tr('Payload missing — capture the file again.'),
                              ),
                            ),
                          );
                          return;
                        }
                        if (!context.mounted) return;
                        context.push('/consent/${job.id}');
                      },
                      child: Text(
                        job.status == UploadJobStatus.failed
                            ? context.tr('Retry upload')
                            : context.tr('Continue to consent'),
                      ),
                    ),
                    const SizedBox(width: 8),
                    TextButton(
                      onPressed: () async {
                        final ok = await showDialog<bool>(
                          context: context,
                          builder: (ctx) => AlertDialog(
                            title: Text(context.tr('Discard this artefact?')),
                            content: Text(
                              context.tr('Removes the local copy and job record from this phone.'),
                            ),
                            actions: [
                              TextButton(
                                onPressed: () => Navigator.pop(ctx, false),
                                child: Text(context.tr('Cancel')),
                              ),
                              FilledButton(
                                onPressed: () => Navigator.pop(ctx, true),
                                child: Text(context.tr('Discard')),
                              ),
                            ],
                          ),
                        );
                        if (ok == true) await queue.discardJob(job.id);
                      },
                      child: Text(
                        context.tr('Discard'),
                        style: TextStyle(color: theme.colorScheme.error),
                      ),
                    ),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }

  String _statusLabel(BuildContext context, UploadJobStatus status) => switch (status) {
    UploadJobStatus.pendingConsent => context.tr('Waiting for consent'),
    UploadJobStatus.queued => context.tr('Queued'),
    UploadJobStatus.encrypting => context.tr('Encrypting'),
    UploadJobStatus.uploading => context.tr('Uploading'),
    UploadJobStatus.complete => context.tr('Sent'),
    UploadJobStatus.failed => context.tr('Failed'),
  };

  IconData _iconFor(UploadJobStatus status) => switch (status) {
    UploadJobStatus.complete => Icons.check_circle_outline,
    UploadJobStatus.failed => Icons.error_outline,
    UploadJobStatus.pendingConsent => Icons.privacy_tip_outlined,
    _ => Icons.cloud_upload_outlined,
  };
}
