import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../config/theme_preference.dart';
import '../l10n/app_localizations.dart';

/// Compact theme menu (light / dark / system). Prefer [SettingsButton] in app bars.
class ThemePickerButton extends StatelessWidget {
  const ThemePickerButton({super.key});

  @override
  Widget build(BuildContext context) {
    final preference = context.watch<ThemePreference>();
    return PopupMenuButton<ThemeMode>(
      tooltip: context.tr('Choose light, dark, or system theme'),
      initialValue: preference.mode,
      onSelected: preference.select,
      icon: const Icon(Icons.brightness_6_outlined),
      itemBuilder: (context) => [
        for (final mode in const [
          ThemeMode.system,
          ThemeMode.light,
          ThemeMode.dark,
        ])
          PopupMenuItem(
            value: mode,
            child: Row(
              children: [
                Icon(_iconFor(mode), size: 18),
                const SizedBox(width: 10),
                Text(context.tr(_labelFor(mode))),
                if (preference.mode == mode) ...[
                  const SizedBox(width: 8),
                  const Icon(Icons.check, size: 18),
                ],
              ],
            ),
          ),
      ],
    );
  }

  static IconData _iconFor(ThemeMode mode) => switch (mode) {
    ThemeMode.dark => Icons.dark_mode_outlined,
    ThemeMode.light => Icons.light_mode_outlined,
    ThemeMode.system => Icons.brightness_auto_outlined,
  };

  static String _labelFor(ThemeMode mode) => switch (mode) {
    ThemeMode.dark => 'Dark',
    ThemeMode.light => 'Light',
    ThemeMode.system => 'System',
  };
}
