import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../l10n/app_localizations.dart';
import '../models/models.dart';
import '../services/upload_queue.dart';

/// Persistent banner while an artefact is encrypting/uploading in the background.
class UploadProgressBanner extends StatelessWidget {
  const UploadProgressBanner({super.key});

  @override
  Widget build(BuildContext context) {
    final queue = context.watch<UploadQueue>();
    UploadJob? job = queue.activeUploadJob;
    if (job == null) {
      for (final j in queue.jobs) {
        if (j.status == UploadJobStatus.encrypting ||
            j.status == UploadJobStatus.uploading) {
          job = j;
          break;
        }
      }
    }
    if (job == null) return const SizedBox.shrink();

    final theme = Theme.of(context);
    final encrypting = job.status == UploadJobStatus.encrypting;
    final label = encrypting
        ? context.tr('Encrypting in the background…')
        : context.tr('Uploading in the background…');
    final pct = encrypting ? null : job.progress;

    return Material(
      color: theme.colorScheme.primaryContainer,
      borderRadius: BorderRadius.circular(14),
      child: InkWell(
        borderRadius: BorderRadius.circular(14),
        onTap: () => context.push('/my-data'),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  SizedBox(
                    width: 22,
                    height: 22,
                    child: CircularProgressIndicator(
                      strokeWidth: 2.4,
                      value: pct == null || pct == 0 ? null : pct,
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          label,
                          style: theme.textTheme.titleSmall?.copyWith(
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        Text(
                          context.tr(job.artefact.label),
                          style: theme.textTheme.bodySmall?.copyWith(
                            color: theme.colorScheme.onPrimaryContainer
                                .withValues(alpha: 0.85),
                          ),
                        ),
                      ],
                    ),
                  ),
                  if (pct != null && pct > 0)
                    Text(
                      '${(pct * 100).round()}%',
                      style: theme.textTheme.labelLarge?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                ],
              ),
              const SizedBox(height: 8),
              ClipRRect(
                borderRadius: BorderRadius.circular(999),
                child: LinearProgressIndicator(
                  minHeight: 4,
                  value: pct == null || pct == 0 ? null : pct,
                ),
              ),
              const SizedBox(height: 6),
              Text(
                context.tr('You can leave the app — tap for details'),
                style: theme.textTheme.labelSmall?.copyWith(
                  color: theme.colorScheme.onPrimaryContainer
                      .withValues(alpha: 0.75),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
