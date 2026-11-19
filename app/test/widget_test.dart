import 'package:flutter_test/flutter_test.dart';

import 'package:collectemoi_app/config/tenant_config.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('tenant config parses bundled json shape', () {
    const raw = {
      'id': 'default',
      'display_name': 'CollecteMoi',
      'server_base_url': 'http://127.0.0.1:8080',
      'tls_pins_spki_sha256': <String>[],
      'case_encryption_public_key_hex': '',
      'enabled_collectors': ['android_bugreport', 'message_report'],
      'support_contact': 'a@b.c',
      'privacy_policy_url': 'https://example',
      'default_locale': 'en',
      'mode': 'generic',
    };
    final tenant = TenantConfig.fromJson(raw);
    expect(tenant.displayName, 'CollecteMoi');
    expect(tenant.hasAndroidBugreport, isTrue);
    expect(tenant.hasIosSysdiagnose, isFalse);
  });
}
