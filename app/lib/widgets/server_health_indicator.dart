import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';

import '../l10n/app_localizations.dart';
import '../services/server_health.dart';

/// Red / green status light for lab API `/healthz` reachability.
/// Tap opens details (including the last contact exception when offline).
class ServerHealthIndicator extends StatelessWidget {
  const ServerHealthIndicator({super.key});

  @override
  Widget build(BuildContext context) {
    final health = context.watch<ServerHealth>();
    final (color, tooltip) = switch (health.status) {
      ServerReachability.online => (
          const Color(0xFF22C55E),
          context.tr('Server online'),
        ),
      ServerReachability.offline => (
          const Color(0xFFEF4444),
          context.tr('Server offline'),
        ),
      ServerReachability.checking => (
          const Color(0xFF9CA3AF),
          context.tr('Checking server…'),
        ),
      ServerReachability.unknown => (
          const Color(0xFF9CA3AF),
          context.tr('Server status unknown'),
        ),
    };

    return IconButton(
      tooltip: tooltip,
      onPressed: () => _showStatus(context),
      icon: Icon(Icons.circle, size: 14, color: color),
    );
  }

  Future<void> _showStatus(BuildContext context) async {
    final health = context.read<ServerHealth>();
    // Refresh once when opening so the dialog shows a current probe.
    await health.check();
    if (!context.mounted) return;

    final theme = Theme.of(context);
    final statusLabel = switch (health.status) {
      ServerReachability.online => context.tr('Server online'),
      ServerReachability.offline => context.tr('Server offline'),
      ServerReachability.checking => context.tr('Checking server…'),
      ServerReachability.unknown => context.tr('Server status unknown'),
    };
    final checked = health.lastCheckedAt;
    final checkedLabel = checked == null
        ? context.tr('Not checked yet')
        : checked.toLocal().toString().split('.').first;
    final url = (health.lastUrl ?? '').trim();
    final error = (health.lastError ?? '').trim();

    await showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      isScrollControlled: true,
      builder: (sheetContext) {
        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(20, 4, 20, 20),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(
                  context.tr('Server status'),
                  style: theme.textTheme.titleLarge?.copyWith(
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: 12),
                ListTile(
                  contentPadding: EdgeInsets.zero,
                  leading: Icon(
                    health.isOnline
                        ? Icons.check_circle_outline
                        : Icons.error_outline,
                    color: health.isOnline
                        ? const Color(0xFF22C55E)
                        : const Color(0xFFEF4444),
                  ),
                  title: Text(statusLabel),
                  subtitle: Text(
                    context
                        .tr('Last check: {when}')
                        .replaceAll('{when}', checkedLabel),
                  ),
                ),
                if (url.isNotEmpty) ...[
                  const SizedBox(height: 4),
                  Text(
                    context.tr('Endpoint'),
                    style: theme.textTheme.labelLarge,
                  ),
                  const SizedBox(height: 4),
                  SelectableText(
                    url,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      fontFamily: 'monospace',
                    ),
                  ),
                ],
                if (error.isNotEmpty) ...[
                  const SizedBox(height: 16),
                  Text(
                    context.tr('Error details'),
                    style: theme.textTheme.labelLarge?.copyWith(
                      color: theme.colorScheme.error,
                    ),
                  ),
                  const SizedBox(height: 6),
                  Container(
                    width: double.infinity,
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      color: theme.colorScheme.errorContainer
                          .withValues(alpha: 0.35),
                      borderRadius: BorderRadius.circular(12),
                      border: Border.all(
                        color: theme.colorScheme.error.withValues(alpha: 0.35),
                      ),
                    ),
                    child: SelectableText(
                      error,
                      style: theme.textTheme.bodySmall?.copyWith(
                        fontFamily: 'monospace',
                        height: 1.35,
                        color: theme.colorScheme.onSurface,
                      ),
                    ),
                  ),
                ] else if (health.isOnline) ...[
                  const SizedBox(height: 12),
                  Text(
                    context.tr('The lab API answered /healthz successfully.'),
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
                const SizedBox(height: 20),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    FilledButton.tonal(
                      onPressed: () async {
                        await health.check();
                        if (sheetContext.mounted) {
                          Navigator.of(sheetContext).pop();
                        }
                        if (context.mounted) {
                          await _showStatus(context);
                        }
                      },
                      child: Text(context.tr('Check again')),
                    ),
                    if (error.isNotEmpty)
                      OutlinedButton.icon(
                        onPressed: () async {
                          await Clipboard.setData(ClipboardData(text: error));
                          if (!sheetContext.mounted) return;
                          ScaffoldMessenger.of(sheetContext).showSnackBar(
                            SnackBar(
                              content: Text(context.tr('Error copied')),
                            ),
                          );
                        },
                        icon: const Icon(Icons.copy, size: 18),
                        label: Text(context.tr('Copy error')),
                      ),
                    TextButton(
                      onPressed: () => Navigator.of(sheetContext).pop(),
                      child: Text(context.tr('Close')),
                    ),
                  ],
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}
