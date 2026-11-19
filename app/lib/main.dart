import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import 'app_router.dart';
import 'config/app_theme.dart';
import 'l10n/app_localizations.dart';
import 'config/lab_http_overrides_stub.dart'
    if (dart.library.io) 'config/lab_http_overrides_io.dart';
import 'config/server_url.dart';
import 'config/theme_preference.dart';
import 'config/tenant_config.dart';
import 'services/api_client.dart';
import 'services/envelope_service.dart';
import 'services/session_store.dart';
import 'services/upload_queue.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  installLabHttpOverrides();

  final bundled = await TenantConfig.loadBundled();
  final serverUrl = resolveServerBaseUrl(bundled);
  final tenant = bundled.copyWith(serverBaseUrl: serverUrl);
  final themeConfig = await ThemeConfig.loadBundled();
  final themePreference = await ThemePreference.load();
  final store = SessionStore();
  final api = ApiClient(baseUrl: tenant.serverBaseUrl);
  final envelope = EnvelopeService(labSealBaseUrl: tenant.serverBaseUrl);
  final queue = UploadQueue(store: store, api: api, envelope: envelope);
  await queue.load();

  final refresh = SessionRefresh(queue);
  final router = createRouter(refresh);

  runApp(
    MultiProvider(
      providers: [
        Provider.value(value: tenant),
        Provider.value(value: themeConfig),
        ChangeNotifierProvider.value(value: themePreference),
        Provider.value(value: api),
        ChangeNotifierProvider.value(value: queue),
      ],
      child: CollecteMoiApp(
        title: tenant.displayName,
        themeConfig: themeConfig,
        routerConfig: router,
      ),
    ),
  );
}

class CollecteMoiApp extends StatelessWidget {
  const CollecteMoiApp({
    super.key,
    required this.title,
    required this.themeConfig,
    required this.routerConfig,
  });

  final String title;
  final ThemeConfig themeConfig;
  final RouterConfig<Object> routerConfig;

  @override
  Widget build(BuildContext context) {
    final themePreference = context.watch<ThemePreference>();
    final defaultLocale = context.watch<TenantConfig>().defaultLocale;
    return MaterialApp.router(
      title: title,
      supportedLocales: appSupportedLocales,
      localeResolutionCallback: (deviceLocale, supportedLocales) {
        Locale? findLanguage(String? languageCode) {
          if (languageCode == null || languageCode.isEmpty) return null;
          for (final locale in supportedLocales) {
            if (locale.languageCode == languageCode.toLowerCase()) {
              return locale;
            }
          }
          return null;
        }

        return findLanguage(deviceLocale?.languageCode) ??
            findLanguage(defaultLocale.split(RegExp('[-_]')).first) ??
            const Locale('en');
      },
      theme: buildTenantTheme(themeConfig),
      darkTheme: buildTenantTheme(themeConfig, brightness: Brightness.dark),
      themeMode: themePreference.mode,
      routerConfig: routerConfig,
    );
  }
}
