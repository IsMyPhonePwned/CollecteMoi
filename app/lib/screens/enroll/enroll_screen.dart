import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:provider/provider.dart';

import '../../config/tenant_config.dart';
import '../../crypto/noise_wrap.dart';
import '../../l10n/app_localizations.dart';
import '../../models/models.dart';
import '../../services/api_client.dart';
import '../../services/device_keys.dart';
import '../../services/upload_queue.dart';
import '../../widgets/theme_picker_button.dart';

class EnrollScreen extends StatefulWidget {
  const EnrollScreen({super.key});

  @override
  State<EnrollScreen> createState() => _EnrollScreenState();
}

class _EnrollScreenState extends State<EnrollScreen> {
  final _emailController = TextEditingController(
    text: kIsWeb ? 'analyst@example.com' : '',
  );
  final _labKeyController = TextEditingController(
    text: kIsWeb ? 'LAB-DEMO01' : '',
  );
  final _otpController = TextEditingController();
  TextEditingController? _serverController;
  bool _scanning = false;
  bool _busy = false;
  String? _error;
  String? _hint;
  String? _challengeId;
  String? _pendingEmail;
  String? _pendingCompanyId;

  TextEditingController get _server {
    return _serverController ??= TextEditingController(
      text: context.read<TenantConfig>().serverBaseUrl,
    );
  }

  @override
  void dispose() {
    _emailController.dispose();
    _labKeyController.dispose();
    _otpController.dispose();
    _serverController?.dispose();
    super.dispose();
  }

  Future<void> _start() async {
    setState(() {
      _busy = true;
      _error = null;
      _hint = null;
    });
    try {
      final serverUrl = _server.text.trim().replaceAll(RegExp(r'/$'), '');
      if (serverUrl.isEmpty) throw Exception(context.tr('Server URL is required'));
      final api = context.read<ApiClient>()..baseUrl = serverUrl;
      final labKey = _labKeyController.text.trim();
      final email = _emailController.text.trim();
      if (labKey.isEmpty && email.isEmpty) {
        throw Exception(context.tr('Enter a work email or a lab key'));
      }
      final res = await api.enrollStart(
        email: labKey.isEmpty ? email : null,
        labKey: labKey.isNotEmpty ? labKey : null,
      );
      final labOtp = res['lab_otp'] as String?;
      if (labOtp != null && labOtp.isNotEmpty) {
        _otpController.text = labOtp;
      }
      setState(() {
        _challengeId = res['challenge_id'] as String?;
        _pendingEmail = res['email'] as String?;
        _pendingCompanyId = res['company_id'] as String?;
        _hint = labOtp != null
            ? context.tr('Lab OTP for {email}: {code}')
                .replaceAll('{email}', '${res['email_masked']}')
                .replaceAll('{code}', labOtp)
            : context.tr('Code sent to {email} ({company})')
                .replaceAll('{email}', '${res['email_masked']}')
                .replaceAll('{company}', '${res['company_name']}');
      });
    } catch (e) {
      if (mounted) setState(() => _error = e.toString());
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _confirm() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final challengeId = _challengeId;
      if (challengeId == null || challengeId.isEmpty) {
        throw Exception(context.tr('Request a code first'));
      }
      final code = _otpController.text.trim();
      if (code.isEmpty) throw Exception(context.tr('Enter the verification code'));

      final serverUrl = _server.text.trim().replaceAll(RegExp(r'/$'), '');
      final tenant = context.read<TenantConfig>();
      final api = context.read<ApiClient>()..baseUrl = serverUrl;
      final keys = SoftDeviceKeyPair.generate();
      final noiseKeyHex = NoiseWrap.generateKeyHex();
      final platform = kIsWeb
          ? 'web'
          : switch (defaultTargetPlatform) {
              TargetPlatform.iOS => 'ios',
              TargetPlatform.android => 'android',
              _ => 'other',
            };

      final res = await api.enrollConfirm(
        challengeId: challengeId,
        code: code,
        devicePublicKeyHex: keys.publicKeyHex,
        noiseKeyHex: noiseKeyHex,
        platform: platform,
        model: kIsWeb ? 'browser' : defaultTargetPlatform.name,
        osVersion: kIsWeb ? 'web' : defaultTargetPlatform.name,
      );

      final session = EnrollmentSession(
        deviceId: res['device_id'] as String,
        caseId: res['case_id'] as String,
        devicePublicKeyHex: keys.publicKeyHex,
        devicePrivateKeyHex: keys.privateKeyHex,
        casePublicKeyHex:
            (res['case_public_key'] as String?) ??
            tenant.caseEncryptionPublicKeyHex,
        serverBaseUrl: serverUrl,
        displayName: (res['display_name'] as String?) ?? tenant.displayName,
        noiseKeyHex: noiseKeyHex,
        noiseKeyFingerprint: res['noise_key_fingerprint'] as String?,
        email: _pendingEmail,
        companyId: _pendingCompanyId,
      );

      if (!mounted) return;
      await context.read<UploadQueue>().setSession(session);
      if (!mounted) return;
      context.go('/');
    } catch (e) {
      if (mounted) setState(() => _error = e.toString());
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final tenant = context.watch<TenantConfig>();
    final theme = Theme.of(context);

    return Scaffold(
      body: SafeArea(
        child: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 480),
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const Align(
                    alignment: Alignment.centerRight,
                    child: ThemePickerButton(),
                  ),
                  const Spacer(),
                  if (tenant.hasLogo) ...[
                    Center(
                      child: Image.asset(
                        tenant.logoAsset!,
                        height: 72,
                        fit: BoxFit.contain,
                        errorBuilder: (context, error, stackTrace) =>
                            const SizedBox.shrink(),
                      ),
                    ),
                    const SizedBox(height: 20),
                  ],
                  Text(
                    tenant.displayName,
                    style: theme.textTheme.displaySmall?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  if (tenant.tagline.isNotEmpty) ...[
                    const SizedBox(height: 8),
                    Text(
                      context.tr(tenant.tagline),
                      style: theme.textTheme.titleMedium?.copyWith(
                        color: theme.colorScheme.primary,
                      ),
                    ),
                  ],
                  const SizedBox(height: 12),
                  Text(
                    context.tr(tenant.enrollLead),
                    style: theme.textTheme.bodyLarge?.copyWith(height: 1.4),
                  ),
                  const SizedBox(height: 28),
                  TextField(
                    controller: _server,
                    enabled: !_busy,
                    decoration: InputDecoration(
                      labelText: context.tr('Server URL'),
                      border: OutlineInputBorder(),
                      hintText: 'https://127.0.0.1:8080',
                    ),
                    keyboardType: TextInputType.url,
                  ),
                  const SizedBox(height: 12),
                  TextField(
                    controller: _emailController,
                    enabled: !_busy,
                    decoration: InputDecoration(
                      labelText: context.tr('Work email'),
                      border: OutlineInputBorder(),
                      hintText: 'you@example.com',
                    ),
                    keyboardType: TextInputType.emailAddress,
                  ),
                  const SizedBox(height: 12),
                  TextField(
                    controller: _labKeyController,
                    enabled: !_busy,
                    decoration: InputDecoration(
                      labelText: context.tr('Lab key (optional)'),
                      border: OutlineInputBorder(),
                      hintText: 'LAB-DEMO01',
                    ),
                  ),
                  if (!kIsWeb) ...[
                    const SizedBox(height: 12),
                    if (_scanning)
                      SizedBox(
                        height: 200,
                        child: ClipRRect(
                          borderRadius: BorderRadius.circular(16),
                          child: MobileScanner(
                            onDetect: (capture) {
                              final value =
                                  capture.barcodes.firstOrNull?.rawValue;
                              if (value != null && !_busy) {
                                setState(() {
                                  _scanning = false;
                                  _labKeyController.text = value.trim();
                                });
                              }
                            },
                          ),
                        ),
                      )
                    else
                      FilledButton.tonalIcon(
                        onPressed: _busy
                            ? null
                            : () => setState(() => _scanning = true),
                        icon: const Icon(Icons.qr_code_scanner),
                        label: Text(context.tr('Scan lab key QR')),
                      ),
                  ],
                  const SizedBox(height: 12),
                  FilledButton.tonal(
                    onPressed: _busy ? null : _start,
                    child: Text(context.tr('Send verification code')),
                  ),
                  const SizedBox(height: 16),
                  TextField(
                    controller: _otpController,
                    enabled: !_busy,
                    decoration: InputDecoration(
                      labelText: context.tr('Verification code'),
                      border: OutlineInputBorder(),
                    ),
                    keyboardType: TextInputType.number,
                    onSubmitted: (_) => _confirm(),
                  ),
                  if (_hint != null) ...[
                    const SizedBox(height: 8),
                    Text(_hint!, style: theme.textTheme.bodySmall),
                  ],
                  if (_error != null) ...[
                    const SizedBox(height: 12),
                    Text(
                      _error!,
                      style: TextStyle(color: theme.colorScheme.error),
                    ),
                  ],
                  const Spacer(),
                  FilledButton(
                    onPressed: _busy ? null : _confirm,
                    child: _busy
                        ? const SizedBox(
                            height: 22,
                            width: 22,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : Text(context.tr('Confirm & enroll')),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
