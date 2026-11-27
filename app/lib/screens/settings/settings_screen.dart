import 'package:flutter/material.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:provider/provider.dart';

import '../../config/app_settings.dart';
import '../../config/tenant_config.dart';
import '../../config/theme_preference.dart';
import '../../l10n/app_localizations.dart';
import '../../services/server_health.dart';
import '../../services/upload_queue.dart';
import '../../widgets/server_health_indicator.dart';

class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  String? _appVersion;
  TextEditingController? _serverController;
  String? _serverError;

  @override
  void initState() {
    super.initState();
    _loadVersion();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _serverController ??= TextEditingController(
      text: context.read<AppSettings>().serverBaseUrl,
    );
  }

  @override
  void dispose() {
    _serverController?.dispose();
    super.dispose();
  }

  Future<void> _loadVersion() async {
    try {
      final info = await PackageInfo.fromPlatform();
      if (!mounted) return;
      final build = info.buildNumber.trim();
      setState(() {
        _appVersion = build.isEmpty
            ? info.version
            : '${info.version} ($build)';
      });
    } catch (_) {}
  }

  Future<void> _saveServerUrl() async {
    final raw = _serverController?.text.trim() ?? '';
    final normalized = raw.replaceAll(RegExp(r'/$'), '');
    if (normalized.isEmpty) {
      setState(() => _serverError = context.tr('Server URL is required'));
      return;
    }
    final uri = Uri.tryParse(normalized);
    if (uri == null || !uri.hasScheme || uri.host.isEmpty) {
      setState(
        () => _serverError = context.tr('Enter a valid server URL (https://…)'),
      );
      return;
    }
    setState(() => _serverError = null);
    await context.read<AppSettings>().setServerBaseUrl(normalized);
    if (!mounted) return;
    context.read<UploadQueue>().applyServerBaseUrl(normalized);
    await context.read<ServerHealth>().check();
    if (!mounted) return;
    _serverController?.text = normalized;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(context.tr('Server URL saved'))),
    );
  }

  @override
  Widget build(BuildContext context) {
    final settings = context.watch<AppSettings>();
    final themePreference = context.watch<ThemePreference>();
    final queue = context.watch<UploadQueue>();
    final tenant = context.watch<TenantConfig>();
    final theme = Theme.of(context);
    final serverController = _serverController!;
    final session = queue.session;
    final labName = (session?.displayName.trim().isNotEmpty ?? false)
        ? session!.displayName.trim()
        : tenant.displayName;
    final companyName = session?.companyName?.trim();

    return Scaffold(
      appBar: AppBar(
        title: Text(context.tr('Settings')),
        actions: const [ServerHealthIndicator()],
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
        children: [
          if (session != null) ...[
            Text(context.tr('Lab'), style: theme.textTheme.titleMedium),
            const SizedBox(height: 4),
            Text(
              context.tr(
                'The lab name comes from your company branding on the server and cannot be changed in the app.',
              ),
              style: theme.textTheme.bodySmall,
            ),
            const SizedBox(height: 12),
            ListTile(
              contentPadding: EdgeInsets.zero,
              leading: Icon(
                Icons.apartment_outlined,
                color: theme.colorScheme.primary,
              ),
              title: Text(context.tr('Company')),
              subtitle: Text(
                (companyName != null && companyName.isNotEmpty)
                    ? companyName
                    : (session.companyId?.trim().isNotEmpty == true
                          ? session.companyId!
                          : context.tr('Not set')),
              ),
            ),
            ListTile(
              contentPadding: EdgeInsets.zero,
              leading: Icon(
                Icons.badge_outlined,
                color: theme.colorScheme.primary,
              ),
              title: Text(context.tr('Lab name')),
              subtitle: Text(labName),
            ),
            const SizedBox(height: 28),
          ],
          Text(context.tr('Appearance'), style: theme.textTheme.titleMedium),
          const SizedBox(height: 8),
          Text(
            context.tr('Follows this phone’s light or dark setting by default.'),
            style: theme.textTheme.bodySmall,
          ),
          const SizedBox(height: 12),
          SegmentedButton<ThemeMode>(
            segments: [
              ButtonSegment(
                value: ThemeMode.system,
                label: Text(context.tr('System')),
                icon: const Icon(Icons.brightness_auto_outlined),
              ),
              ButtonSegment(
                value: ThemeMode.light,
                label: Text(context.tr('Light')),
                icon: const Icon(Icons.light_mode_outlined),
              ),
              ButtonSegment(
                value: ThemeMode.dark,
                label: Text(context.tr('Dark')),
                icon: const Icon(Icons.dark_mode_outlined),
              ),
            ],
            selected: {themePreference.mode},
            onSelectionChanged: (modes) {
              if (modes.isEmpty) return;
              themePreference.select(modes.first);
            },
          ),
          const SizedBox(height: 28),
          Text(context.tr('Server'), style: theme.textTheme.titleMedium),
          const SizedBox(height: 4),
          Text(
            context.tr(
              'Change the CollecteMoi API address used for enrollment. Builds generated with a server URL hide this field on the enroll screen.',
            ),
            style: theme.textTheme.bodySmall,
          ),
          const SizedBox(height: 12),
          TextField(
            controller: serverController,
            decoration: InputDecoration(
              labelText: context.tr('Server URL'),
              border: const OutlineInputBorder(),
              hintText: 'https://example.com:8080',
              errorText: _serverError,
            ),
            keyboardType: TextInputType.url,
            textInputAction: TextInputAction.done,
            onSubmitted: (_) => _saveServerUrl(),
          ),
          const SizedBox(height: 8),
          Align(
            alignment: Alignment.centerLeft,
            child: FilledButton.tonal(
              onPressed: _saveServerUrl,
              child: Text(context.tr('Save server URL')),
            ),
          ),
          const SizedBox(height: 28),
          Text(context.tr('Enrollment'), style: theme.textTheme.titleMedium),
          const SizedBox(height: 4),
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            value: settings.showLabKey,
            onChanged: settings.setShowLabKey,
            title: Text(context.tr('Show lab key')),
            subtitle: Text(
              context.tr(
                'For lab testing only. Keep this off for normal enrollment with a work email.',
              ),
            ),
          ),
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            value: settings.trustSelfSigned,
            onChanged: settings.setTrustSelfSigned,
            title: Text(context.tr('Trust self-signed certificate')),
            subtitle: Text(
              context.tr(
                'Allow HTTPS to a lab server that uses a self-signed TLS certificate. Turn this off for production servers with a trusted certificate.',
              ),
            ),
          ),
          if (_appVersion != null) ...[
            const SizedBox(height: 28),
            Text(context.tr('About'), style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            Text(
              context
                  .tr('App version {version}')
                  .replaceAll('{version}', _appVersion!),
              style: theme.textTheme.bodyMedium?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ],
        ],
      ),
    );
  }
}
