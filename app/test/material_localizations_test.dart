import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:collectemoi_app/config/tenant_config.dart';
import 'package:collectemoi_app/config/theme_preference.dart';
import 'package:collectemoi_app/l10n/app_localizations.dart';
import 'package:collectemoi_app/main.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('MaterialApp exposes MaterialLocalizations (date/time format)', (
    tester,
  ) async {
    late String formatted;
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: const [
          GlobalMaterialLocalizations.delegate,
          GlobalWidgetsLocalizations.delegate,
          GlobalCupertinoLocalizations.delegate,
        ],
        supportedLocales: appSupportedLocales,
        home: Builder(
          builder: (context) {
            final l = MaterialLocalizations.of(context);
            final now = DateTime(2026, 11, 23, 14, 5);
            formatted =
                '${l.formatMediumDate(now)} ${l.formatTimeOfDay(TimeOfDay.fromDateTime(now))}';
            return Text(formatted);
          },
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(formatted, isNotEmpty);
    expect(find.text(formatted), findsOneWidget);
  });

  testWidgets('CollecteMoiApp provides MaterialLocalizations to routes', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final themePreference = ThemePreference(prefs, ThemeMode.light);

    final router = GoRouter(
      routes: [
        GoRoute(
          path: '/',
          builder: (context, state) {
            // Same call path as My data submission timestamps.
            final l = MaterialLocalizations.of(context);
            final stamp = l.formatMediumDate(DateTime(2026, 1, 2));
            return Scaffold(body: Text('ok:$stamp'));
          },
        ),
      ],
    );

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          Provider.value(
            value: TenantConfig.fromJson(const {
              'id': 'default',
              'display_name': 'CollecteMoi',
              'server_base_url': 'https://127.0.0.1:8080',
              'tls_pins_spki_sha256': <String>[],
              'case_encryption_public_key_hex': '',
              'enabled_collectors': <String>['android_bugreport'],
              'support_contact': '',
              'privacy_policy_url': '',
              'default_locale': 'en',
              'mode': 'generic',
            }),
          ),
          ChangeNotifierProvider.value(value: themePreference),
        ],
        child: CollecteMoiApp(
          title: 'CollecteMoi',
          themeConfig: ThemeConfig.fromJson(const {
            'seed_color': '#0B126F',
            'light': <String, dynamic>{},
            'dark': <String, dynamic>{},
          }),
          routerConfig: router,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.textContaining('ok:'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
