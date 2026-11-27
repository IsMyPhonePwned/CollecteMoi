import 'dart:io';

/// Controls whether non-loopback self-signed certificates are accepted.
/// Loopback (localhost / 127.0.0.1) is always allowed for local lab use.
class LabTlsSettings {
  LabTlsSettings._();

  static bool trustSelfSigned = false;
}

/// Allow the API's generated self-signed certificate for loopback, and
/// optionally for any host when [LabTlsSettings.trustSelfSigned] is on.
class _LabHttpOverrides extends HttpOverrides {
  @override
  HttpClient createHttpClient(SecurityContext? context) {
    final client = super.createHttpClient(context);
    client.badCertificateCallback = (cert, host, port) {
      if (LabTlsSettings.trustSelfSigned) return true;
      if (host.toLowerCase() == 'localhost') return true;
      final address = InternetAddress.tryParse(host);
      return address?.isLoopback ?? false;
    };
    return client;
  }
}

void installLabHttpOverrides() {
  HttpOverrides.global = _LabHttpOverrides();
}

void applyLabTlsPreference({required bool trustSelfSigned}) {
  LabTlsSettings.trustSelfSigned = trustSelfSigned;
  // Reinstall so any future clients pick up the flag (callback already reads it live).
  HttpOverrides.global = _LabHttpOverrides();
}
