import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:provider/provider.dart';

import '../../config/app_settings.dart';
import '../../config/server_url.dart';
import '../../config/tenant_config.dart';
import '../../crypto/noise_wrap.dart';
import '../../l10n/app_localizations.dart';
import '../../models/models.dart';
import '../../services/api_client.dart';
import '../../services/device_info_service.dart';
import '../../services/device_keys.dart';
import '../../services/upload_queue.dart';
import '../../widgets/server_health_indicator.dart';
import '../../widgets/settings_button.dart';

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
  String? _pendingCompanyName;

  TextEditingController get _server {
    return _serverController ??= TextEditingController(
      text: context.read<AppSettings>().serverBaseUrl,
    );
  }

  /// Prefer Settings / baked URL; only use the enroll field when it is shown.
  String _resolvedServerUrl() {
    if (hasBundledServerUrl) {
      return context.read<AppSettings>().serverBaseUrl.trim().replaceAll(
            RegExp(r'/$'),
            '',
          );
    }
    return _server.text.trim().replaceAll(RegExp(r'/$'), '');
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
      final serverUrl = _resolvedServerUrl();
      if (serverUrl.isEmpty) throw Exception(context.tr('Server URL is required'));
      final settings = context.read<AppSettings>();
      final queue = context.read<UploadQueue>();
      final api = context.read<ApiClient>();
      await settings.setServerBaseUrl(serverUrl);
      if (!mounted) return;
      queue.applyServerBaseUrl(serverUrl);
      api.baseUrl = serverUrl;
      final showLabKey = settings.showLabKey;
      final labKey = showLabKey ? _labKeyController.text.trim() : '';
      final email = _emailController.text.trim();
      if (labKey.isEmpty && email.isEmpty) {
        throw Exception(
          context.tr(
            showLabKey
                ? 'Enter a work email or a lab key'
                : 'Enter a work email',
          ),
        );
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
        _pendingCompanyName = res['company_name'] as String?;
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

      final serverUrl = _resolvedServerUrl();
      if (serverUrl.isEmpty) throw Exception(context.tr('Server URL is required'));
      final settings = context.read<AppSettings>();
      final queue = context.read<UploadQueue>();
      final tenant = context.read<TenantConfig>();
      final api = context.read<ApiClient>();
      await settings.setServerBaseUrl(serverUrl);
      if (!mounted) return;
      queue.applyServerBaseUrl(serverUrl);
      api.baseUrl = serverUrl;
      final keys = SoftDeviceKeyPair.generate();
      final noiseKeyHex = NoiseWrap.generateKeyHex();
      final deviceInfo = await DeviceInfoService.capture();
      final appVersion = await DeviceInfoService.appVersion();
      if (!mounted) return;

      final clientIdentity = ClientIdentity.fromDevice(
        deviceInfo,
        appVersion: appVersion,
      );
      final res = await api.enrollConfirm(
        challengeId: challengeId,
        code: code,
        devicePublicKeyHex: keys.publicKeyHex,
        noiseKeyHex: noiseKeyHex,
        device: clientIdentity,
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
        displayName: (res['display_name'] as String?)?.trim().isNotEmpty == true
            ? (res['display_name'] as String).trim()
            : tenant.displayName,
        noiseKeyHex: noiseKeyHex,
        noiseKeyFingerprint: res['noise_key_fingerprint'] as String?,
        email: _pendingEmail,
        companyId: ((res['company_id'] as String?)?.trim().isNotEmpty == true
                ? res['company_id'] as String
                : _pendingCompanyId)
            ?.trim(),
        companyName: ((res['company_name'] as String?)?.trim().isNotEmpty == true
                ? res['company_name'] as String
                : _pendingCompanyName)
            ?.trim(),
        devicePlatform: deviceInfo.platform,
        deviceModel: deviceInfo.model,
        deviceOsVersion: deviceInfo.osVersion,
        deviceManufacturer: deviceInfo.manufacturer,
        deviceBrand: deviceInfo.brand,
        deviceName: deviceInfo.name,
        deviceDetails: deviceInfo.details,
        appVersion: appVersion,
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
    final showLabKey = context.watch<AppSettings>().showLabKey;
    final theme = Theme.of(context);
    final enrollLead = showLabKey
        ? context.tr(
            'Enter your work email to receive a one-time code, or use a lab key for local testing.',
          )
        : context.brandedEnrollLead();

    return Scaffold(
      // Keep the verification fields above the keyboard on small screens.
      resizeToAvoidBottomInset: true,
      body: SafeArea(
        child: LayoutBuilder(
          builder: (context, constraints) {
            return SingleChildScrollView(
              padding: EdgeInsets.fromLTRB(
                24,
                24,
                24,
                24 + MediaQuery.viewInsetsOf(context).bottom,
              ),
              child: ConstrainedBox(
                constraints: BoxConstraints(
                  minHeight: constraints.maxHeight - 48,
                  maxWidth: 480,
                ),
                child: IntrinsicHeight(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      const Align(
                        alignment: Alignment.centerRight,
                        child: Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            ServerHealthIndicator(),
                            SettingsButton(),
                          ],
                        ),
                      ),
                      const SizedBox(height: 12),
                      if (tenant.hasLogo) ...[
                        Center(
                          child: Image.asset(
                            tenant.logoAsset!,
                            height: 64,
                            width: 220,
                            fit: BoxFit.contain,
                            errorBuilder: (context, error, stackTrace) =>
                                const SizedBox.shrink(),
                          ),
                        ),
                        const SizedBox(height: 16),
                      ],
                      Text(
                        tenant.displayName,
                        style: theme.textTheme.headlineMedium?.copyWith(
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                      if (tenant.tagline.isNotEmpty ||
                          context.brandedTagline().isNotEmpty) ...[
                        const SizedBox(height: 8),
                        Text(
                          context.brandedTagline().isNotEmpty
                              ? context.brandedTagline()
                              : context.tr(tenant.tagline),
                          style: theme.textTheme.titleMedium?.copyWith(
                            color: theme.colorScheme.primary,
                          ),
                        ),
                      ],
                      const SizedBox(height: 12),
                      Text(
                        enrollLead,
                        style: theme.textTheme.bodyLarge?.copyWith(height: 1.4),
                      ),
                      const SizedBox(height: 24),
                      // Baked at APK/IPA build (`--server`); edit under Settings instead.
                      if (!hasBundledServerUrl) ...[
                        TextField(
                          controller: _server,
                          enabled: !_busy,
                          decoration: InputDecoration(
                            labelText: context.tr('Server URL'),
                            border: const OutlineInputBorder(),
                            hintText: 'https://127.0.0.1:8080',
                          ),
                          keyboardType: TextInputType.url,
                        ),
                        const SizedBox(height: 4),
                      ],
                      SwitchListTile(
                        contentPadding: EdgeInsets.zero,
                        value: context.watch<AppSettings>().trustSelfSigned,
                        onChanged: _busy
                            ? null
                            : (v) => context.read<AppSettings>().setTrustSelfSigned(v),
                        title: Text(context.tr('Trust self-signed certificate')),
                        subtitle: Text(
                          context.tr(
                            'Needed for lab servers with a self-signed HTTPS certificate.',
                          ),
                        ),
                      ),
                      const SizedBox(height: 4),
                      TextField(
                        controller: _emailController,
                        enabled: !_busy,
                        decoration: InputDecoration(
                          labelText: context.tr('Work email'),
                          border: const OutlineInputBorder(),
                          hintText: 'you@example.com',
                        ),
                        keyboardType: TextInputType.emailAddress,
                      ),
                      if (showLabKey) ...[
                        const SizedBox(height: 12),
                        TextField(
                          controller: _labKeyController,
                          enabled: !_busy,
                          decoration: InputDecoration(
                            labelText: context.tr('Lab key (optional)'),
                            border: const OutlineInputBorder(),
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
                          border: const OutlineInputBorder(),
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
                      const SizedBox(height: 16),
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
            );
          },
        ),
      ),
    );
  }
}
