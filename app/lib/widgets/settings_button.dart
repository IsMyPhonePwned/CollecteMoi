import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import '../l10n/app_localizations.dart';

class SettingsButton extends StatelessWidget {
  const SettingsButton({super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      tooltip: context.tr('Settings'),
      onPressed: () => context.push('/settings'),
      icon: const Icon(Icons.settings_outlined),
    );
  }
}
