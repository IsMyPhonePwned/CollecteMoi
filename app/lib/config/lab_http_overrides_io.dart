import 'dart:io';

/// Lab helper: accept the API's auto-generated self-signed cert.
class _LabHttpOverrides extends HttpOverrides {
  @override
  HttpClient createHttpClient(SecurityContext? context) {
    final client = super.createHttpClient(context);
    client.badCertificateCallback = (cert, host, port) => true;
    return client;
  }
}

void installLabHttpOverrides() {
  HttpOverrides.global = _LabHttpOverrides();
}
