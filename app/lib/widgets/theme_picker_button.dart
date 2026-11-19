import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../config/theme_preference.dart';
import '../l10n/app_localizations.dart';

class ThemePickerButton extends StatelessWidget {
  const ThemePickerButton({super.key});

  @override
  Widget build(BuildContext context) {
    final preference = context.watch<ThemePreference>();
    return PopupMenuButton<ThemeMode>(
      tooltip: context.tr('Choose light or dark theme'),
      initialValue: preference.mode,
      onSelected: (mode) => preference.select(mode),
      icon: const Icon(Icons.brightness_6_outlined),
      itemBuilder: (context) => [
        for (final mode in [ThemeMode.light, ThemeMode.dark])
          PopupMenuItem(
            value: mode,
            child: Row(
              children: [
                Icon(
                  mode == ThemeMode.dark
                      ? Icons.dark_mode_outlined
                      : Icons.light_mode_outlined,
                  size: 18,
                ),
                const SizedBox(width: 10),
                Text(context.tr(mode == ThemeMode.dark ? 'Dark' : 'Light')),
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
}
