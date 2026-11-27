import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:provider/provider.dart';

import 'app_router.dart';
import 'config/app_theme.dart';
import 'config/lab_http_overrides_stub.dart'
    if (dart.library.io) 'config/lab_http_overrides_io.dart';
import 'config/app_settings.dart';
import 'config/server_url.dart';
import 'config/theme_preference.dart';
import 'config/tenant_config.dart';
import 'services/api_client.dart';
import 'services/envelope_service.dart';
import 'services/server_health.dart';
import 'services/session_store.dart';
import 'services/upload_queue.dart';
import 'widgets/upload_notice_listener.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  installLabHttpOverrides();

  final bundled = await TenantConfig.loadBundled();
  final resolvedDefault = resolveServerBaseUrl(bundled);
  final themeConfig = await ThemeConfig.loadBundled();
  final themePreference = await ThemePreference.load();
  final appSettings =
      await AppSettings.load(defaultServerUrl: resolvedDefault);
  final tenant = bundled.copyWith(serverBaseUrl: appSettings.serverBaseUrl);
  final store = SessionStore();
  final api = ApiClient(baseUrl: appSettings.serverBaseUrl);
  final envelope = EnvelopeService(labSealBaseUrl: appSettings.serverBaseUrl);
  final queue = UploadQueue(
    store: store,
    api: api,
    envelope: envelope,
    tenant: tenant,
  );
  await queue.load();

  final refresh = SessionRefresh(queue);
  final router = createRouter(refresh);

  runApp(
    MultiProvider(
      providers: [
        Provider.value(value: tenant),
        Provider.value(value: themeConfig),
        ChangeNotifierProvider.value(value: themePreference),
        ChangeNotifierProvider.value(value: appSettings),
        Provider.value(value: api),
        ChangeNotifierProvider(create: (_) => ServerHealth(api)),
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
    final tenant = context.watch<TenantConfig>();
    final defaultLocale = tenant.defaultLocale;
    final supported = tenant.supportedLocales;
    return MaterialApp.router(
      title: title,
      // Required for Material widgets (dialogs, date/time format, ScaffoldMessenger…).
      // Without these, MaterialLocalizations.of(context) throws at runtime.
      localizationsDelegates: const [
        GlobalMaterialLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
      ],
      supportedLocales: supported,
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
            findLanguage(supportedLocales.isNotEmpty
                ? supportedLocales.first.languageCode
                : 'en') ??
            const Locale('en');
      },
      theme: buildTenantTheme(themeConfig),
      darkTheme: buildTenantTheme(themeConfig, brightness: Brightness.dark),
      themeMode: themePreference.mode,
      routerConfig: routerConfig,
      builder: (context, child) => UploadNoticeListener(
        child: child ?? const SizedBox.shrink(),
      ),
    );
  }
}
