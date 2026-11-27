import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
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

enum _GuidePath { developer, samsung }

class AndroidBugreportFlow extends StatefulWidget {
  const AndroidBugreportFlow({super.key});

  @override
  State<AndroidBugreportFlow> createState() => _AndroidBugreportFlowState();
}

class _AndroidBugreportFlowState extends State<AndroidBugreportFlow>
    with WidgetsBindingObserver {
  final _bridge = PlatformBridge();
  _GuidePath _path = _GuidePath.developer;
  int _step = 1;
  bool _devEnabled = false;
  DateTime? _waitStarted;
  String? _receivedPath;
  Uint8List? _pickedBytes;
  String? _validationHint;
  bool _picking = false;

  int get _total => _path == _GuidePath.samsung ? 6 : 7;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _refreshDevState();
    _pullPending();
    _bridge.shareEvents.listen((event) {
      final path = event['path']?.toString();
      if (path != null && mounted) {
        setState(() {
          _receivedPath = path;
          _pickedBytes = null;
          _validationHint = null;
          // Land on the “file ready” step for the active path.
          _step = _path == _GuidePath.samsung ? 5 : 6;
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
      _pullPending();
    }
  }

  Future<void> _pullPending() async {
    final jobs = await _bridge.pendingShareJobs();
    if (jobs.isEmpty || !mounted) return;
    final path = jobs.first['path']?.toString();
    if (path != null) {
      setState(() {
        _receivedPath = path;
        _pickedBytes = null;
        _validationHint = null;
        _step = _path == _GuidePath.samsung ? 5 : 6;
      });
    }
  }

  Future<void> _refreshDevState() async {
    final enabled = await _bridge.isDeveloperOptionsEnabled();
    if (!mounted) return;
    setState(() {
      _devEnabled = enabled;
      if (_path == _GuidePath.developer && enabled && _step == 2) {
        _step = 3;
      }
    });
  }

  void _startSamsungGuide() {
    setState(() {
      _path = _GuidePath.samsung;
      _step = 1;
      _waitStarted = null;
      _receivedPath = null;
      _pickedBytes = null;
      _validationHint = null;
    });
  }

  void _startDeveloperGuide() {
    setState(() {
      _path = _GuidePath.developer;
      _step = _devEnabled ? 3 : 2;
      _waitStarted = null;
      _receivedPath = null;
      _pickedBytes = null;
      _validationHint = null;
    });
  }

  Future<void> _pickDumpstateOrBugreport() async {
    if (_picking) return;
    setState(() => _picking = true);
    try {
      final file = await FilePicker.pickFile(
        type: FileType.custom,
        allowedExtensions: const ['zip', 'txt', 'gz', 'log'],
      );
      if (file == null || !mounted) return;
      final bytes = await file.readAsBytes();
      if (!mounted) return;
      final check = validateAndroidBugreport(bytes);
      if (!check.ok) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text(
              context
                  .tr('This does not look like a bugreport:\n{details}')
                  .replaceAll('{details}', check.messages.join('\n')),
            ),
          ),
        );
        return;
      }
      setState(() {
        _receivedPath = file.name;
        _pickedBytes = bytes;
        _validationHint = check.captureHint;
        _step = _path == _GuidePath.samsung ? 5 : 6;
      });
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            context
                .tr('Could not open file: {error}')
                .replaceAll('{error}', '$e'),
          ),
        ),
      );
    } finally {
      if (mounted) setState(() => _picking = false);
    }
  }

  Future<void> _submit() async {
    final bytes = _pickedBytes ?? await readPathBytes(_receivedPath!);
    if (!mounted) return;
    final check = validateAndroidBugreport(bytes);
    if (!check.ok) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            context
                .tr('This does not look like a bugreport:\n{details}')
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

  Future<void> _continue() async {
    if (_path == _GuidePath.samsung) {
      await _continueSamsung();
      return;
    }
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
          await _pickDumpstateOrBugreport();
          return;
        }
        setState(() => _step = 7);
      case 7:
        await _submit();
    }
  }

  Future<void> _continueSamsung() async {
    switch (_step) {
      case 1:
        setState(() => _step = 2);
      case 2:
        // User opens Phone app and dials; we cannot open the secret code for them.
        setState(() => _step = 3);
      case 3:
        setState(() {
          _waitStarted ??= DateTime.now();
          _step = 4;
        });
      case 4:
        setState(() => _step = 5);
      case 5:
        if (_receivedPath == null) {
          await _pickDumpstateOrBugreport();
          return;
        }
        setState(() => _step = 6);
      case 6:
        await _submit();
    }
  }

  @override
  Widget build(BuildContext context) {
    context.watch<TenantConfig>();
    final guideTitle = context.branded((c) => c.androidBugreport.title);
    final guide = GuideCopy(
      title: guideTitle,
      headline: context.branded((c) => c.androidBugreport.headline),
      body: context.branded((c) => c.androidBugreport.body),
    );
    return GuideScaffold(
      title: guideTitle,
      step: _step,
      totalSteps: _total,
      primaryLabel: _primaryLabel(context),
      onPrimary: _picking ? null : _continue,
      secondaryLabel: _secondaryLabel(context),
      onSecondary: _secondaryAction(),
      child: _path == _GuidePath.samsung
          ? _samsungBody(context)
          : _developerBody(context, guide),
    );
  }

  String _primaryLabel(BuildContext context) {
    if (_path == _GuidePath.samsung) {
      return switch (_step) {
        1 => context.tr('Start Samsung guide'),
        2 => context.tr('I dialed *#9900#'),
        3 => context.tr('I started dumpstate'),
        4 => context.tr('Copy to sdcard finished'),
        5 => _receivedPath == null
            ? context.tr('Select dumpstate from /log')
            : context.tr('Continue to consent'),
        6 => context.tr('Review and send'),
        _ => context.tr('Continue'),
      };
    }
    return switch (_step) {
      1 => context.tr('Check this phone'),
      2 => context.tr('Open device info'),
      3 => context.tr('Open developer settings'),
      4 => context.tr('I started the bug report'),
      5 => context.tr('I got the notification'),
      6 => _receivedPath == null
          ? context.tr('Select a previous bugreport')
          : context.tr('Continue to consent'),
      7 => context.tr('Review and send'),
      _ => context.tr('Continue'),
    };
  }

  String? _secondaryLabel(BuildContext context) {
    if (_path == _GuidePath.samsung) {
      if (_step == 1) {
        return context.tr('Use Developer options instead');
      }
      if (_step == 5 && _receivedPath != null) {
        return context.tr('Choose a different dumpstate');
      }
      return null;
    }
    if (_step == 1) {
      return context.tr('Samsung dumpstate guide (no Developer options)');
    }
    if (_step == 2) return context.tr('I already enabled Developer options');
    if (_step == 5) return context.tr('Select a previous bugreport');
    if (_step >= 6 && _receivedPath != null) {
      return context.tr('Choose a different bugreport');
    }
    return null;
  }

  VoidCallback? _secondaryAction() {
    if (_path == _GuidePath.samsung) {
      if (_step == 1) return _startDeveloperGuide;
      if (_step == 5 && _receivedPath != null) {
        return _picking ? null : _pickDumpstateOrBugreport;
      }
      return null;
    }
    if (_step == 1) return _startSamsungGuide;
    if (_step == 2) return () => setState(() => _step = 3);
    if (_step == 5 || (_step >= 6 && _receivedPath != null)) {
      return _picking ? null : _pickDumpstateOrBugreport;
    }
    return null;
  }

  Widget _developerBody(BuildContext context, GuideCopy guide) {
    return switch (_step) {
      1 => GuideBody(
          headline: guide.headline,
          body:
              '${guide.body}\n\n'
              '${context.tr('On Samsung phones without Developer options, use the Samsung dumpstate guide instead.')}',
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
          body: context.tr(
            'In Developer options, tap Take bug report and choose Full report (Interactive drops useful sections). Generation takes a few minutes.',
          ),
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
          body:
              '${context.tr('Tap the notification, accept the privacy warning, and pick CollecteMoi in the share sheet.')} '
              '${context.tr('Already have a bugreport ZIP from earlier? Use “Select a previous bugreport” below — Files / Downloads often keep bugreport-*.zip.')}',
          illustration: _HintCard(
            icon: Icons.ios_share,
            text: context.tr('Share sheet → CollecteMoi, or pick a saved ZIP'),
          ),
        ),
      6 => _fileReadyBody(context),
      _ => GuideBody(
          headline: context.tr('Almost done'),
          body: context.tr(
            'Optional: you can turn Developer options off again after sending. Continue to the consent screen to encrypt and upload.',
          ),
        ),
    };
  }

  Widget _samsungBody(BuildContext context) {
    return switch (_step) {
      1 => GuideBody(
          headline: context.tr('Samsung dumpstate (no Developer options)'),
          body: context.tr(
            'This path uses Samsung’s dialer code to write a dumpstate to storage. You do not need Developer options or “Take bug report”.',
          ),
          illustration: _HintCard(
            icon: Icons.dialpad,
            text: context.tr(
              'Samsung — dial *#9900# → dumpstate → Copy to sdcard → /log',
            ),
          ),
        ),
      2 => GuideBody(
          headline: context.tr('Open the Phone dialer'),
          body: context.tr(
            'Open the Phone / Dialer app (not this app). Type *#9900# and open the SysDump / dumpstate menu that appears. Menus differ slightly by One UI version.',
          ),
          illustration: _HintCard(
            icon: Icons.phone_in_talk_outlined,
            text: context.tr('Phone → dial *#9900#'),
          ),
        ),
      3 => GuideBody(
          headline: context.tr('Run dumpstate'),
          body: context.tr(
            'In the menu, choose Run dumpstate/logcat (wording may be SysDump, Dumpstate, or similar). Wait while Samsung collects logs — keep the screen on.',
          ),
          illustration: _HintCard(
            icon: Icons.bug_report_outlined,
            text: context.tr('*#9900# → Run dumpstate/logcat'),
          ),
        ),
      4 => GuideBody(
          headline: context.tr('Copy to sdcard'),
          body:
              '${context.tr('When dumpstate finishes, choose Copy to sdcard (or Copy to storage). Samsung writes files under Internal storage → log (/log).')} '
              '${_waitStarted == null ? '' : context.tr('Timer started.')}',
          illustration: _HintCard(
            icon: Icons.sd_card_outlined,
            text: context.tr('Copy to sdcard → Internal storage / log'),
          ),
        ),
      5 => _fileReadyBody(
          context,
          waitingHeadline: context.tr('Select the dumpstate file'),
          waitingBody: context.tr(
            'Open Files (or My Files) → Internal storage → log. Pick the newest dumpstate / log file (often dumpstate_*.log or similar), then continue.',
          ),
          readyHeadline: context.tr('Dumpstate received'),
        ),
      _ => GuideBody(
          headline: context.tr('Almost done'),
          body: context.tr(
            'Next we ask for your consent, then encrypt and upload this dumpstate like a normal Android bugreport.',
          ),
        ),
    };
  }

  Widget _fileReadyBody(
    BuildContext context, {
    String? waitingHeadline,
    String? waitingBody,
    String? readyHeadline,
  }) {
    final waiting = _receivedPath == null;
    return GuideBody(
      headline: waiting
          ? (waitingHeadline ?? context.tr('Waiting for the shared file…'))
          : (readyHeadline ?? context.tr('Bugreport received')),
      body: waiting
          ? (waitingBody ??
              context.tr(
                'Leave this screen open for a new share, or select a previous bugreport ZIP / dumpstate from Downloads or Files.',
              ))
          : () {
              final base = context
                  .tr(
                    'File ready:\n{path}\n\nNext we will show what is inside and ask for your consent before uploading.',
                  )
                  .replaceAll('{path}', _receivedPath!);
              final hint = _validationHint?.trim();
              if (hint == null || hint.isEmpty) return base;
              return '$base\n\n$hint';
            }(),
      illustration: _picking
          ? const Padding(
              padding: EdgeInsets.symmetric(vertical: 12),
              child: Center(child: CircularProgressIndicator()),
            )
          : waiting
              ? _HintCard(
                  icon: Icons.folder_open,
                  text: context.tr('Files → Internal storage → log'),
                )
              : null,
    );
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
        color: Theme.of(context)
            .colorScheme
            .primaryContainer
            .withValues(alpha: 0.45),
        borderRadius: BorderRadius.circular(16),
      ),
      child: Row(
        children: [
          Icon(icon, size: 36),
          const SizedBox(width: 16),
          Expanded(
            child: Text(text, style: Theme.of(context).textTheme.titleMedium),
          ),
        ],
      ),
    );
  }
}
