import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

class ThemePreference extends ChangeNotifier {
  ThemePreference(this._preferences, this.mode);

  static const _storageKey = 'collectemoi-app-theme-mode';
  final SharedPreferences _preferences;
  ThemeMode mode;

  static Future<ThemePreference> load() async {
    final preferences = await SharedPreferences.getInstance();
    final saved = preferences.getString(_storageKey);
    return ThemePreference(
      preferences,
      saved == 'dark' ? ThemeMode.dark : ThemeMode.light,
    );
  }

  Future<void> select(ThemeMode value) async {
    if (value == ThemeMode.system || value == mode) return;
    mode = value;
    notifyListeners();
    await _preferences.setString(
      _storageKey,
      value == ThemeMode.dark ? 'dark' : 'light',
    );
  }
}
