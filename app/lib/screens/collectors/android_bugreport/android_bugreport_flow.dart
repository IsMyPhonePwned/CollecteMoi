import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../../config/tenant_config.dart';
import '../../../l10n/app_localizations.dart';
import '../../../models/models.dart';
import '../../../services/bugreport_validator.dart';
import '../../../services/file_bytes.dart';
import '../../../services/platform_bridge.dart';
import '../../../services/upload_queue.dart';
import '../../../widgets/guide_scaffold.dart';

class AndroidBugreportFlow extends StatefulWidget {
  const AndroidBugreportFlow({super.key});

  @override
  State<AndroidBugreportFlow> createState() => _AndroidBugreportFlowState();
}

class _AndroidBugreportFlowState extends State<AndroidBugreportFlow>
    with WidgetsBindingObserver {
  final _bridge = PlatformBridge();
  int _step = 1;
  bool _devEnabled = false;
  DateTime? _waitStarted;
  String? _receivedPath;

  static const total = 7;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _refreshDevState();
    _bridge.shareEvents.listen((event) {
      final path = event['path']?.toString();
      if (path != null && mounted) {
        setState(() {
          _receivedPath = path;
          _step = 6;
        });
      }
    });
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      _refreshDevState();
    }
  }

  Future<void> _refreshDevState() async {
    final enabled = await _bridge.isDeveloperOptionsEnabled();
    if (!mounted) return;
    setState(() {
      _devEnabled = enabled;
      if (enabled && _step == 2) _step = 3;
    });
  }

  Future<void> _continue() async {
    switch (_step) {
      case 1:
        await _refreshDevState();
        setState(() => _step = _devEnabled ? 3 : 2);
      case 2:
        await _bridge.openDeviceInfoSettings();
      case 3:
        await _bridge.openDeveloperSettings();
        setState(() => _step = 4);
      case 4:
        setState(() {
          _waitStarted ??= DateTime.now();
          _step = 5;
        });
      case 5:
        setState(() => _step = 6);
      case 6:
        if (_receivedPath == null) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text(
                context.tr('Waiting for a shared bugreport. Use the system share sheet and pick CollecteMoi.'),
              ),
            ),
          );
          return;
        }
        setState(() => _step = 7);
      case 7:
        final bytes = await readPathBytes(_receivedPath!);
        if (!mounted) return;
        final check = validateAndroidBugreport(bytes);
        if (!check.ok) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text(
                context.tr('This does not look like a bugreport:\n{details}')
                    .replaceAll('{details}', check.messages.join('\n')),
              ),
            ),
          );
          return;
        }
        final job = await context.read<UploadQueue>().enqueueBytes(
              artefact: ArtefactKind.androidBugreport,
              bytes: bytes,
              label: _receivedPath!,
            );
        if (!mounted) return;
        context.push('/consent/${job.id}');
    }
  }

  @override
  Widget build(BuildContext context) {
    final guide = context.watch<TenantConfig>().copy.androidBugreport;
    return GuideScaffold(
      title: context.tr(guide.title),
      step: _step,
      totalSteps: total,
      primaryLabel: _primaryLabel(context),
      onPrimary: _continue,
      secondaryLabel: _step == 2 ? context.tr('I already enabled Developer options') : null,
      onSecondary: () => setState(() => _step = 3),
      child: _body(context, guide),
    );
  }

  String _primaryLabel(BuildContext context) => switch (_step) {
        1 => context.tr('Check this phone'),
        2 => context.tr('Open device info'),
        3 => context.tr('Open developer settings'),
        4 => context.tr('I started the bug report'),
        5 => context.tr('I got the notification'),
        6 => context.tr('Continue to consent'),
        7 => context.tr('Review and send'),
        _ => context.tr('Continue'),
      };

  Widget _body(BuildContext context, GuideCopy guide) {
    return switch (_step) {
      1 => GuideBody(
          headline: context.tr(guide.headline),
          body: context.tr(guide.body),
        ),
      2 => GuideBody(
          headline: context.tr('Enable Developer options'),
          body: '${context.tr('Open the screen below, find Build number, and tap it seven times until you see the confirmation.')} '
              '${context.tr('Manufacturer menus differ (Pixel, Samsung, Xiaomi…).')} '
              '${context.tr(_devEnabled ? 'Developer options are already on.' : 'We will detect when it turns on.')}',
          illustration: _HintCard(
            icon: Icons.touch_app,
            text: context.tr('Settings → About phone → Build number × 7'),
          ),
        ),
      3 => GuideBody(
          headline: context.tr('Take a Full bug report'),
          body: context.tr('In Developer options, tap Take bug report and choose Full report (Interactive drops useful sections). Generation takes a few minutes.'),
          illustration: _HintCard(
            icon: Icons.bug_report_outlined,
            text: context.tr('Developer options → Take bug report → Full'),
          ),
        ),
      4 => GuideBody(
          headline: context.tr('Wait for the system'),
          body:
              '${context.tr('Keep the phone unlocked and on power if you can.')} '
              '${context.tr('A notification will change to “Bug report captured”.')} '
              '${_waitStarted == null ? '' : context.tr('Timer started.')}',
          illustration: const _HintCard(
            icon: Icons.hourglass_top,
            text: 'Usually 2–10 minutes',
          ),
        ),
      5 => GuideBody(
          headline: context.tr('Share into CollecteMoi'),
          body: context.tr('Tap the notification, accept the privacy warning, and pick CollecteMoi in the share sheet. You do not need to find the file yourself.'),
        ),
      6 => GuideBody(
          headline: _receivedPath == null
              ? context.tr('Waiting for the shared file…')
              : context.tr('Bugreport received'),
          body: _receivedPath == null
              ? context.tr('Leave this screen open. When Android shares the ZIP, we will pick it up automatically.')
              : context.tr('File ready at:\n{path}\n\nNext we will show what is inside and ask for your consent before uploading.').replaceAll('{path}', _receivedPath!),
        ),
      _ => GuideBody(
          headline: context.tr('Almost done'),
          body: context.tr('Optional: you can turn Developer options off again after sending. Continue to the consent screen to encrypt and upload.'),
        ),
    };
  }
}

class _HintCard extends StatelessWidget {
  const _HintCard({required this.icon, required this.text});
  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(20),
      decoration: BoxDecoration(
        color: Theme.of(context).colorScheme.primaryContainer.withValues(alpha: 0.45),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Row(
        children: [
          Icon(icon, size: 36),
          const SizedBox(width: 16),
          Expanded(child: Text(text, style: Theme.of(context).textTheme.titleMedium)),
        ],
      ),
    );
  }
}
