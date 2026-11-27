import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../l10n/app_localizations.dart';
import '../services/upload_queue.dart';

/// Shows an in-app SnackBar when [UploadQueue.latestCompletionNotice] is set.
class UploadNoticeListener extends StatefulWidget {
  const UploadNoticeListener({super.key, required this.child});

  final Widget child;

  @override
  State<UploadNoticeListener> createState() => _UploadNoticeListenerState();
}

class _UploadNoticeListenerState extends State<UploadNoticeListener> {
  UploadQueue? _queue;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final queue = context.read<UploadQueue>();
    if (!identical(_queue, queue)) {
      _queue?.removeListener(_onQueue);
      _queue = queue;
      _queue!.addListener(_onQueue);
    }
  }

  @override
  void dispose() {
    _queue?.removeListener(_onQueue);
    super.dispose();
  }

  void _onQueue() {
    final notice = _queue?.latestCompletionNotice;
    if (notice == null || !mounted) return;
    final messenger = ScaffoldMessenger.maybeOf(context);
    if (messenger == null) return;
    final label = context.tr(notice.artefactLabel);
    final text = notice.success
        ? context
            .tr('{artefact} uploaded successfully')
            .replaceAll('{artefact}', label)
        : context
            .tr('{artefact} upload failed')
            .replaceAll('{artefact}', label);
    messenger.showSnackBar(
      SnackBar(
        content: Text(text),
        backgroundColor: notice.success ? null : Theme.of(context).colorScheme.error,
        behavior: SnackBarBehavior.floating,
        duration: const Duration(seconds: 4),
      ),
    );
    _queue?.clearCompletionNotice();
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
