import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../config/tenant_config.dart';
import '../../l10n/app_localizations.dart';
import '../../models/models.dart';
import '../../services/api_client.dart';
import '../../services/upload_queue.dart';
import '../../util/format_bytes.dart';
import '../../widgets/server_health_indicator.dart';
import '../../widgets/settings_button.dart';
import '../../widgets/upload_progress_banner.dart';

class MyDataScreen extends StatefulWidget {
  const MyDataScreen({super.key});

  @override
  State<MyDataScreen> createState() => _MyDataScreenState();
}

class _MyDataScreenState extends State<MyDataScreen> {
  @override
  Widget build(BuildContext context) {
    final queue = context.watch<UploadQueue>();
    context.watch<TenantConfig>();
    final copyTitle = context.branded((c) => c.myData.title);
    final copyDeviceHeading = context.branded((c) => c.myData.deviceHeading);
    final copyArtefactsHeading =
        context.branded((c) => c.myData.artefactsHeading);
    final copyEmpty = context.branded((c) => c.myData.empty);
    final copyClearButton = context.branded((c) => c.myData.clearButton);
    final session = queue.session;
    final theme = Theme.of(context);
    final canPop = context.canPop();

    return Scaffold(
      appBar: AppBar(
        title: Text(copyTitle),
        leading: IconButton(
          tooltip: context.tr('Back to home'),
          icon: Icon(canPop ? Icons.arrow_back : Icons.home_outlined),
          onPressed: () {
            if (canPop) {
              context.pop();
            } else {
              context.go('/');
            }
          },
        ),
        actions: const [ServerHealthIndicator(), SettingsButton()],
      ),
      body: ListView(
        padding: const EdgeInsets.all(20),
        children: [
          const UploadProgressBanner(),
          if (queue.hasBackgroundUpload) const SizedBox(height: 14),
          if (session != null) ...[
            Text(copyDeviceHeading, style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            _DeviceInfoCard(session: session),
            const SizedBox(height: 20),
            _ServerHistory(session: session),
            const SizedBox(height: 20),
          ],
          Text(context.tr('On this device'), style: theme.textTheme.titleLarge),
          const SizedBox(height: 4),
          Text(copyArtefactsHeading, style: theme.textTheme.bodyMedium),
          const SizedBox(height: 8),
          if (queue.jobs.any((j) => j.status == UploadJobStatus.failed)) ...[
            Card(
              color: theme.colorScheme.errorContainer.withValues(alpha: 0.35),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      context.tr(
                        'Some items did not reach the lab. You can resend them without capturing again, or delete them from this phone.',
                      ),
                    ),
                    const SizedBox(height: 10),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        if (!queue.busy &&
                            queue.jobs.any((j) => j.canResendFast))
                          FilledButton(
                            onPressed: () async {
                              await queue.resendAllFailed();
                              if (!context.mounted) return;
                              ScaffoldMessenger.of(context).showSnackBar(
                                SnackBar(
                                  content: Text(
                                    context.tr('Resending in the background…'),
                                  ),
                                ),
                              );
                            },
                            child: Text(context.tr('Resend all')),
                          ),
                        OutlinedButton(
                          onPressed: () async {
                            final ok = await showDialog<bool>(
                              context: context,
                              builder: (ctx) => AlertDialog(
                                title: Text(context.tr('Delete all failed items?')),
                                content: Text(
                                  context.tr(
                                    'Removes failed messages and files from this phone only. Nothing already received by the lab is erased.',
                                  ),
                                ),
                                actions: [
                                  TextButton(
                                    onPressed: () => Navigator.pop(ctx, false),
                                    child: Text(context.tr('Cancel')),
                                  ),
                                  FilledButton(
                                    onPressed: () => Navigator.pop(ctx, true),
                                    child: Text(context.tr('Delete all failed')),
                                  ),
                                ],
                              ),
                            );
                            if (ok != true) return;
                            final n = await queue.discardAllFailed();
                            if (!context.mounted) return;
                            ScaffoldMessenger.of(context).showSnackBar(
                              SnackBar(
                                content: Text(
                                  context
                                      .tr('Deleted {count} failed item(s)')
                                      .replaceAll('{count}', '$n'),
                                ),
                              ),
                            );
                          },
                          child: Text(
                            context.tr('Delete all failed'),
                            style: TextStyle(color: theme.colorScheme.error),
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 10),
          ],
          if (queue.jobs.isEmpty) Text(copyEmpty),
          if (queue.jobs.any((j) => j.status == UploadJobStatus.failed)) ...[
            Text(
              context
                  .tr('Failed ({count})')
                  .replaceAll(
                    '{count}',
                    '${queue.jobs.where((j) => j.status == UploadJobStatus.failed).length}',
                  ),
              style: theme.textTheme.titleMedium?.copyWith(
                color: theme.colorScheme.error,
              ),
            ),
            const SizedBox(height: 8),
            ...(() {
              final failed = queue.jobs
                  .where((j) => j.status == UploadJobStatus.failed)
                  .toList()
                ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
              return failed.map((job) => _JobCard(job: job));
            })(),
            const SizedBox(height: 16),
          ],
          if (queue.jobs.any((j) => j.status != UploadJobStatus.failed)) ...[
            if (queue.jobs.any((j) => j.status == UploadJobStatus.failed))
              Text(
                context.tr('Other uploads'),
                style: theme.textTheme.titleMedium,
              ),
            if (queue.jobs.any((j) => j.status == UploadJobStatus.failed))
              const SizedBox(height: 8),
            ...(() {
              final other = queue.jobs
                  .where((j) => j.status != UploadJobStatus.failed)
                  .toList()
                ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
              return other.map((job) => _JobCard(job: job));
            })(),
          ],
          const SizedBox(height: 24),
          OutlinedButton(
            onPressed: session == null
                ? null
                : () async {
                    final ok = await showDialog<bool>(
                      context: context,
                      builder: (ctx) => AlertDialog(
                        title: Text('$copyClearButton?'),
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
            child: Text(copyClearButton),
          ),
        ],
      ),
    );
  }
}

class _DeviceInfoCard extends StatefulWidget {
  const _DeviceInfoCard({required this.session});

  final EnrollmentSession session;

  @override
  State<_DeviceInfoCard> createState() => _DeviceInfoCardState();
}

class _DeviceInfoCardState extends State<_DeviceInfoCard> {
  List<ServerSubmission>? _submissions;
  bool _loadingSubs = true;

  @override
  void initState() {
    super.initState();
    _loadSubmissions();
  }

  @override
  void didUpdateWidget(covariant _DeviceInfoCard oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.session.deviceId != widget.session.deviceId) {
      _loadSubmissions();
    }
  }

  Future<void> _loadSubmissions() async {
    setState(() => _loadingSubs = true);
    try {
      final submissions = await context.read<ApiClient>().clientSubmissions(
            deviceId: widget.session.deviceId,
            noiseKeyHex: widget.session.noiseKeyHex,
          );
      if (!mounted) return;
      setState(() => _submissions = submissions);
    } catch (_) {
      if (!mounted) return;
      setState(() => _submissions = null);
    } finally {
      if (mounted) setState(() => _loadingSubs = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final session = widget.session;
    final queue = context.watch<UploadQueue>();
    final serverLast = _newestSubmission(_submissions);
    final localLast = _newestLocalJob(queue.jobs);
    final lastAt = _later(serverLast?.submittedAt, localLast?.createdAt);
    final lastArtefact = serverLast != null &&
            (localLast == null ||
                !localLast.createdAt.isAfter(serverLast.submittedAt))
        ? serverLast.artefact.label
        : localLast?.artefact.label;
    final submissionCount = _submissions?.length;

    final companyLabel = (session.companyName ?? '').trim().isNotEmpty
        ? session.companyName!.trim()
        : ((session.companyId ?? '').trim().isNotEmpty
              ? session.companyId!.trim()
              : '');
    final rows = <(String, String)>[
      (context.tr('Lab name'), session.displayName),
      if ((session.email ?? '').isNotEmpty)
        (context.tr('Email'), session.email!),
      if (companyLabel.isNotEmpty) (context.tr('Company'), companyLabel),
      (context.tr('Device ID'), session.deviceId),
      (context.tr('Case'), session.caseId),
      if ((session.appVersion ?? '').isNotEmpty)
        (context.tr('App version'), session.appVersion!),
      if (_loadingSubs)
        (context.tr('Last submission'), context.tr('Loading…'))
      else if (lastAt != null) ...[
        (
          context.tr('Last submission'),
          _localizedDateTime(context, lastAt),
        ),
        if (lastArtefact != null)
          (context.tr('Last artefact'), context.tr(lastArtefact)),
        if (submissionCount != null)
          (
            context.tr('Submissions'),
            '$submissionCount',
          ),
      ] else
        (context.tr('Last submission'), context.tr('None yet')),
      if ((session.devicePlatform ?? '').isNotEmpty)
        (context.tr('Platform'), session.devicePlatform!),
      if ((session.deviceManufacturer ?? '').isNotEmpty)
        (context.tr('Manufacturer'), session.deviceManufacturer!),
      if ((session.deviceBrand ?? '').isNotEmpty &&
          session.deviceBrand != session.deviceManufacturer)
        (context.tr('Brand'), session.deviceBrand!),
      if ((session.deviceModel ?? '').isNotEmpty)
        (context.tr('Model'), session.deviceModel!),
      if ((session.deviceName ?? '').isNotEmpty)
        (context.tr('Device name'), session.deviceName!),
      if ((session.deviceOsVersion ?? '').isNotEmpty)
        (context.tr('OS / system'), session.deviceOsVersion!),
      ..._orderedDetailRows(context, session.deviceDetails),
    ];

    return Card(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 14, 16, 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (session.hasDeviceInfo) ...[
              Text(
                session.deviceSummary,
                style: theme.textTheme.titleSmall?.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                context.tr('Captured at enrollment on this phone.'),
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: 12),
            ] else ...[
              Text(
                context.tr(
                  'Device details were not stored for this enrollment. Re-enroll to capture them.',
                ),
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: 12),
            ],
            for (final row in rows) ...[
              _InfoRow(label: row.$1, value: row.$2),
              const SizedBox(height: 6),
            ],
          ],
        ),
      ),
    );
  }

  static ServerSubmission? _newestSubmission(List<ServerSubmission>? list) {
    if (list == null || list.isEmpty) return null;
    return list.reduce(
      (a, b) => a.submittedAt.isAfter(b.submittedAt) ? a : b,
    );
  }

  static UploadJob? _newestLocalJob(List<UploadJob> jobs) {
    final done = jobs
        .where(
          (j) =>
              j.status == UploadJobStatus.complete ||
              j.uploadId != null,
        )
        .toList();
    if (done.isEmpty) return null;
    return done.reduce((a, b) => a.createdAt.isAfter(b.createdAt) ? a : b);
  }

  static DateTime? _later(DateTime? a, DateTime? b) {
    if (a == null) return b;
    if (b == null) return a;
    return a.isAfter(b) ? a : b;
  }

  static bool _isNoisyDetail(String key) {
    // Fingerprint / UA can be huge; keep the card readable.
    return key == 'fingerprint' || key == 'user_agent' || key == 'sdk';
  }

  static const _detailPriority = <String>[
    'security_patch',
    'api_level',
    'android_release',
    'ios_version',
    'build_id',
    'build_incremental',
    'build_display',
    'build_type',
    'abis',
    'board',
    'bootloader',
    'hardware',
    'product',
    'physical',
  ];

  static List<(String, String)> _orderedDetailRows(
    BuildContext context,
    Map<String, String> details,
  ) {
    final keys = details.keys.where((k) => !_isNoisyDetail(k)).toList()
      ..sort((a, b) {
        final ia = _detailPriority.indexOf(a);
        final ib = _detailPriority.indexOf(b);
        if (ia >= 0 && ib >= 0) return ia.compareTo(ib);
        if (ia >= 0) return -1;
        if (ib >= 0) return 1;
        return a.compareTo(b);
      });
    return [
      for (final k in keys)
        if ((details[k] ?? '').trim().isNotEmpty)
          (_detailLabel(context, k), details[k]!.trim()),
    ];
  }

  static String _detailLabel(BuildContext context, String key) => switch (key) {
    'security_patch' => context.tr('Security patch'),
    'api_level' => context.tr('API level'),
    'sdk' => context.tr('Android SDK'),
    'android_release' => context.tr('Android release'),
    'build_incremental' => context.tr('Build incremental'),
    'build_id' => context.tr('Build ID'),
    'build_display' => context.tr('Build display'),
    'display' => context.tr('Build display'),
    'build_type' => context.tr('Build type'),
    'build_tags' => context.tr('Build tags'),
    'build_host' => context.tr('Build host'),
    'base_os' => context.tr('Base OS'),
    'codename' => context.tr('Codename'),
    'board' => context.tr('Board'),
    'bootloader' => context.tr('Bootloader'),
    'abis' => context.tr('ABIs'),
    'physical' => context.tr('Physical device'),
    'product' => context.tr('Product'),
    'hardware' => context.tr('Hardware'),
    'system' => context.tr('OS name'),
    'ios_version' => context.tr('iOS version'),
    'model_id' => context.tr('Model id'),
    'model_name' => context.tr('Model name'),
    'localized_model' => context.tr('Localized model'),
    'machine' => context.tr('Machine'),
    'darwin_release' => context.tr('Darwin release'),
    'darwin_version' => context.tr('Darwin version'),
    'idfv' => context.tr('Vendor ID'),
    'vendor' => context.tr('Browser vendor'),
    'language' => context.tr('Language'),
    _ => key,
  };
}

class _InfoRow extends StatelessWidget {
  const _InfoRow({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: 120,
          child: Text(
            label,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ),
        Expanded(
          child: SelectableText(
            value,
            style: theme.textTheme.bodyMedium?.copyWith(
              fontFamily: value.length > 24 ? 'monospace' : null,
            ),
          ),
        ),
      ],
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
            final size = formatBytes(submission.sizeBytes);
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

String _jobWhen(BuildContext context, DateTime value) {
  final localizations = MaterialLocalizations.of(context);
  final local = value.toLocal();
  final now = DateTime.now();
  final sameDay =
      local.year == now.year && local.month == now.month && local.day == now.day;
  final time = localizations.formatTimeOfDay(TimeOfDay.fromDateTime(local));
  if (sameDay) return time;
  return '${localizations.formatMediumDate(local)} $time';
}

class _JobCard extends StatelessWidget {
  const _JobCard({required this.job});

  final UploadJob job;

  @override
  Widget build(BuildContext context) {
    final queue = context.read<UploadQueue>();
    final theme = Theme.of(context);
    final size = job.sizeBytes == null ? null : formatBytes(job.sizeBytes);

    final failed = job.status == UploadJobStatus.failed;
    final when = _jobWhen(context, job.createdAt);

    return Card(
      margin: const EdgeInsets.only(bottom: 10),
      color: failed
          ? theme.colorScheme.errorContainer.withValues(alpha: 0.4)
          : null,
      shape: failed
          ? RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(12),
              side: BorderSide(
                color: theme.colorScheme.error.withValues(alpha: 0.55),
              ),
            )
          : null,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 10, 8, 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ListTile(
              contentPadding: EdgeInsets.zero,
              leading: Icon(
                _iconFor(job.status),
                color: failed ? theme.colorScheme.error : null,
              ),
              title: Row(
                children: [
                  Expanded(
                    child: Text(
                      context.tr(job.artefact.label),
                      style: theme.textTheme.titleSmall?.copyWith(
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                  ),
                  if (failed)
                    Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 8,
                        vertical: 2,
                      ),
                      decoration: BoxDecoration(
                        color: theme.colorScheme.error,
                        borderRadius: BorderRadius.circular(999),
                      ),
                      child: Text(
                        context.tr('Failed'),
                        style: theme.textTheme.labelSmall?.copyWith(
                          color: theme.colorScheme.onError,
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                    ),
                ],
              ),
              subtitle: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const SizedBox(height: 4),
                  Text(
                    [
                      if (!failed) _statusLabel(context, job.status),
                      when,
                      ?size,
                    ].join(' · '),
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                  if (failed && (job.error ?? '').trim().isNotEmpty) ...[
                    const SizedBox(height: 6),
                    Text(
                      job.error!.trim(),
                      maxLines: 4,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.error,
                      ),
                    ),
                  ],
                ],
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
            if (job.canResumeConsent || job.status == UploadJobStatus.failed)
              Padding(
                padding: const EdgeInsets.only(left: 8, right: 8, bottom: 4),
                child: Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    if (job.canResendFast && !queue.busy)
                      FilledButton(
                        onPressed: () async {
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
                          await queue.resend(job.id);
                          if (!context.mounted) return;
                          ScaffoldMessenger.of(context).showSnackBar(
                            SnackBar(
                              content: Text(context.tr('Resending in the background…')),
                              duration: const Duration(seconds: 3),
                            ),
                          );
                        },
                        child: Text(context.tr('Resend')),
                      )
                    else if (job.canResumeConsent &&
                        !job.canResendFast &&
                        !queue.busy)
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
                    TextButton(
                      onPressed: () async {
                        final ok = await showDialog<bool>(
                          context: context,
                          builder: (ctx) => AlertDialog(
                            title: Text(
                              failed
                                  ? context.tr('Delete this failed item?')
                                  : context.tr('Discard this artefact?'),
                            ),
                            content: Text(
                              failed
                                  ? context.tr(
                                      'Removes this message or file from this phone. The lab is not changed.',
                                    )
                                  : context.tr(
                                      'Removes the local copy and job record from this phone.',
                                    ),
                            ),
                            actions: [
                              TextButton(
                                onPressed: () => Navigator.pop(ctx, false),
                                child: Text(context.tr('Cancel')),
                              ),
                              FilledButton(
                                onPressed: () => Navigator.pop(ctx, true),
                                child: Text(
                                  failed
                                      ? context.tr('Delete')
                                      : context.tr('Discard'),
                                ),
                              ),
                            ],
                          ),
                        );
                        if (ok == true) {
                          await queue.discardJob(job.id);
                          if (!context.mounted) return;
                          ScaffoldMessenger.of(context).showSnackBar(
                            SnackBar(
                              content: Text(
                                failed
                                    ? context.tr('Failed item deleted')
                                    : context.tr('Item discarded'),
                              ),
                            ),
                          );
                        }
                      },
                      child: Text(
                        failed ? context.tr('Delete') : context.tr('Discard'),
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
