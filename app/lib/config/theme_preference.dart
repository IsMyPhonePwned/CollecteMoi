import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

class ThemePreference extends ChangeNotifier {
  ThemePreference(this._preferences, this.mode);

  static const _storageKey = 'collectemoi-app-theme-mode';
  final SharedPreferences _preferences;

  /// Defaults to [ThemeMode.system] so the app follows the device setting.
  ThemeMode mode;

  static Future<ThemePreference> load() async {
    final preferences = await SharedPreferences.getInstance();
    final saved = preferences.getString(_storageKey);
    return ThemePreference(preferences, _decode(saved));
  }

  static ThemeMode _decode(String? saved) {
    return switch (saved) {
      'dark' => ThemeMode.dark,
      'light' => ThemeMode.light,
      // Missing key, legacy empty, or explicit "system".
      _ => ThemeMode.system,
    };
  }

  static String _encode(ThemeMode mode) {
    return switch (mode) {
      ThemeMode.dark => 'dark',
      ThemeMode.light => 'light',
      ThemeMode.system => 'system',
    };
  }

  Future<void> select(ThemeMode value) async {
    if (value == mode) return;
    mode = value;
    notifyListeners();
    await _preferences.setString(_storageKey, _encode(value));
  }
}
