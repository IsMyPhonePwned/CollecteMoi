import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../config/tenant_config.dart';
import '../../models/models.dart';
import '../../l10n/app_localizations.dart';
import '../../services/api_client.dart';
import '../../services/upload_queue.dart';
import '../../widgets/theme_picker_button.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  List<CollectionRequest> _requests = [];
  String? _error;

  bool get _isAndroid =>
      !kIsWeb && defaultTargetPlatform == TargetPlatform.android;
  bool get _isIos => !kIsWeb && defaultTargetPlatform == TargetPlatform.iOS;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final api = context.read<ApiClient>();
      final requests = await api.requests();
      if (!mounted) return;
      setState(() {
        _requests = requests;
        _error = null;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() => _error = e.toString());
    }
  }

  @override
  Widget build(BuildContext context) {
    final tenant = context.watch<TenantConfig>();
    final queue = context.watch<UploadQueue>();
    final session = queue.session;
    final theme = Theme.of(context);

    return Scaffold(
      appBar: AppBar(
        title: Text(session?.displayName ?? tenant.displayName),
        actions: [
          const ThemePickerButton(),
          IconButton(
            tooltip: context.tr('Submission history and My data'),
            onPressed: () => context.push('/my-data'),
            icon: const Icon(Icons.folder_outlined),
          ),
        ],
      ),
      body: RefreshIndicator(
        onRefresh: _load,
        child: ListView(
          padding: const EdgeInsets.all(20),
          children: [
            Container(
              padding: const EdgeInsets.all(20),
              decoration: BoxDecoration(
                color: theme.colorScheme.surface,
                borderRadius: BorderRadius.circular(18),
                border: Border.all(
                  color: theme.colorScheme.outlineVariant.withValues(
                    alpha: 0.65,
                  ),
                ),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Container(
                    padding: const EdgeInsets.all(10),
                    decoration: BoxDecoration(
                      color: theme.colorScheme.primaryContainer,
                      borderRadius: BorderRadius.circular(12),
                    ),
                    child: Icon(
                      Icons.shield_outlined,
                      color: theme.colorScheme.onPrimaryContainer,
                      size: 22,
                    ),
                  ),
                  const SizedBox(height: 16),
                  Text(
                    kIsWeb ? context.tr('Lab client') : context.tr(tenant.copy.home.headline),
                    style: theme.textTheme.headlineSmall?.copyWith(
                      fontWeight: FontWeight.w700,
                      letterSpacing: -0.4,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    kIsWeb
                        ? context.tr('Enrolled against {url}. Use Lab file upload to push a test artefact through the same chunked protocol.')
                              .replaceAll('{url}', tenant.serverBaseUrl)
                        : context.tr(tenant.copy.home.body),
                    style: theme.textTheme.bodyMedium?.copyWith(
                      height: 1.5,
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
            if (_error != null) ...[
              const SizedBox(height: 12),
              Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
            ],
            if (session != null) ...[
              const SizedBox(height: 16),
              Card(
                child: ListTile(
                  leading: Icon(
                    Icons.history,
                    color: theme.colorScheme.primary,
                  ),
                  title: Text(context.tr('Submission history')),
                  subtitle: Text(
                    context.tr('View uploads received by the lab and their status'),
                  ),
                  trailing: const Icon(Icons.chevron_right),
                  onTap: () => context.push('/my-data'),
                ),
              ),
            ],
            const SizedBox(height: 24),
            if (_requests.isNotEmpty) ...[
              Padding(
                padding: const EdgeInsets.only(bottom: 8, top: 20),
                child: Text(context.tr('Open requests'), style: theme.textTheme.titleLarge),
              ),
              ..._requests.map(
                (r) => Card(
                  child: ListTile(
                    title: Text(r.title),
                    subtitle: Text(
                      r.requestedArtefacts.map((a) => a.label).join(' · '),
                    ),
                    trailing: const Icon(Icons.chevron_right),
                    onTap: () => _openRequest(context, r),
                  ),
                ),
              ),
              const SizedBox(height: 16),
            ],
            if (kIsWeb || (!_isAndroid && !_isIos))
              _CollectorTile(
                icon: Icons.science_outlined,
                title: context.tr('Lab file upload'),
                subtitle: context.tr('Pick any file and submit via the upload API'),
                onTap: () => context.push('/lab/upload'),
              ),
            if (tenant.hasAndroidBugreport && (_isAndroid || kIsWeb))
              _CollectorTile(
                icon: Icons.android,
                title: _isAndroid
                    ? context.tr(tenant.copy.home.androidTitle)
                    : '${context.tr(tenant.copy.home.androidTitle)}${context.tr(' (guide preview)')}',
                subtitle: context.tr(tenant.copy.home.androidSubtitle),
                onTap: () => context.push('/collect/android-bugreport'),
              ),
            if (tenant.hasIosSysdiagnose && (_isIos || kIsWeb))
              _CollectorTile(
                icon: Icons.phone_iphone,
                title: _isIos
                    ? context.tr(tenant.copy.home.iosTitle)
                    : '${context.tr(tenant.copy.home.iosTitle)}${context.tr(' (guide preview)')}',
                subtitle: context.tr(tenant.copy.home.iosSubtitle),
                onTap: () => context.push('/collect/ios-sysdiagnose'),
              ),
            if (tenant.hasMessageReport)
              _CollectorTile(
                icon: Icons.sms_outlined,
                title: context.tr(tenant.copy.home.messageTitle),
                subtitle: context.tr(tenant.copy.home.messageSubtitle),
                onTap: () => context.push('/collect/message'),
              ),
            const SizedBox(height: 24),
            if (queue.jobs.isNotEmpty) ...[
              ListTile(
                contentPadding: EdgeInsets.zero,
                title: Text(
                  context.tr('{count} / {total} uploads finished')
                      .replaceAll('{count}', '${queue.jobs.where((j) => j.status == UploadJobStatus.complete).length}')
                      .replaceAll('{total}', '${queue.jobs.length}'),
                  style: theme.textTheme.labelLarge,
                ),
                subtitle: queue.jobs.any((j) => j.canResumeConsent)
                    ? Text(context.tr('Some uploads need consent or a retry'))
                    : null,
                trailing: TextButton(
                  onPressed: () => context.push('/my-data'),
                  child: Text(context.tr('My data')),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }

  void _openRequest(BuildContext context, CollectionRequest request) {
    final kinds = request.requestedArtefacts;
    if (kinds.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(context.tr('No artefact types on this request'))),
      );
      return;
    }
    final kind = kinds.first;
    final path = switch (kind) {
      ArtefactKind.androidBugreport =>
        (_isAndroid || kIsWeb) ? '/collect/android-bugreport' : null,
      ArtefactKind.iosSysdiagnose =>
        (_isIos || kIsWeb) ? '/collect/ios-sysdiagnose' : null,
      ArtefactKind.messageReport => '/collect/message',
    };
    if (path == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            context.tr('{kind} is not available on this device. Use Lab file upload if you are on web/desktop.')
                .replaceAll('{kind}', context.tr(kind.label)),
          ),
        ),
      );
      if (kIsWeb || (!_isAndroid && !_isIos)) {
        context.push('/lab/upload');
      }
      return;
    }
    context.push(path);
  }
}

class _CollectorTile extends StatelessWidget {
  const _CollectorTile({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: Material(
        color: Theme.of(context).colorScheme.surface,
        borderRadius: BorderRadius.circular(16),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          borderRadius: BorderRadius.circular(16),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.all(16),
            child: Row(
              children: [
                Container(
                  width: 48,
                  height: 48,
                  decoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.primaryContainer,
                    borderRadius: BorderRadius.circular(13),
                  ),
                  child: Icon(
                    icon,
                    size: 24,
                    color: Theme.of(context).colorScheme.onPrimaryContainer,
                  ),
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        style: Theme.of(context).textTheme.titleMedium
                            ?.copyWith(fontWeight: FontWeight.w600),
                      ),
                      const SizedBox(height: 4),
                      Text(
                        subtitle,
                        style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                          color: Theme.of(context).colorScheme.onSurfaceVariant,
                          height: 1.35,
                        ),
                      ),
                    ],
                  ),
                ),
                const Icon(Icons.chevron_right),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
