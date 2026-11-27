import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import 'server_health_indicator.dart';
import 'settings_button.dart';

class GuideScaffold extends StatelessWidget {
  const GuideScaffold({
    super.key,
    required this.title,
    required this.step,
    required this.totalSteps,
    required this.child,
    this.primaryLabel,
    this.onPrimary,
    this.secondaryLabel,
    this.onSecondary,
  });

  final String title;
  final int step;
  final int totalSteps;
  final Widget child;
  final String? primaryLabel;
  final VoidCallback? onPrimary;
  final String? secondaryLabel;
  final VoidCallback? onSecondary;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(
        title: Text(title),
        actions: const [ServerHealthIndicator(), SettingsButton()],
      ),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(20, 8, 20, 20),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                context.tr('Step {step} of {total}')
                    .replaceAll('{step}', '$step')
                    .replaceAll('{total}', '$totalSteps'),
                style: theme.textTheme.labelLarge?.copyWith(
                  color: theme.colorScheme.primary,
                ),
              ),
              const SizedBox(height: 8),
              ClipRRect(
                borderRadius: BorderRadius.circular(999),
                child: LinearProgressIndicator(
                  value: step / totalSteps,
                  minHeight: 6,
                ),
              ),
              const SizedBox(height: 20),
              Expanded(child: child),
              if (secondaryLabel != null) ...[
                TextButton(
                  onPressed: onSecondary,
                  child: Text(secondaryLabel!),
                ),
              ],
              if (primaryLabel != null) ...[
                const SizedBox(height: 8),
                FilledButton(onPressed: onPrimary, child: Text(primaryLabel!)),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class GuideBody extends StatelessWidget {
  const GuideBody({
    super.key,
    required this.headline,
    required this.body,
    this.illustration,
  });

  final String headline;
  final String body;
  final Widget? illustration;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return ListView(
      children: [
        if (illustration != null) ...[
          illustration!,
          const SizedBox(height: 20),
        ],
        Text(headline, style: theme.textTheme.headlineSmall),
        const SizedBox(height: 12),
        Text(body, style: theme.textTheme.bodyLarge?.copyWith(height: 1.45)),
      ],
    );
  }
}
