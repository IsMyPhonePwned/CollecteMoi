import 'package:flutter_test/flutter_test.dart';

import 'package:collectemoi_app/models/models.dart';

void main() {
  test('UploadJob persists payload_path for durable resume', () {
    final job = UploadJob(
      id: 'abc',
      artefact: ArtefactKind.androidBugreport,
      localPath: 'bugreport.zip',
      createdAt: DateTime.utc(2026, 1, 2),
      sizeBytes: 12,
      payloadPath: '/tmp/payloads/abc.bin',
    );
    final roundTrip = UploadJob.fromJson(job.toJson());
    expect(roundTrip.payloadPath, '/tmp/payloads/abc.bin');
    expect(roundTrip.canResumeConsent, isTrue);
    expect(roundTrip.artefact, ArtefactKind.androidBugreport);
  });

  test('completed jobs are not resumable', () {
    final job = UploadJob(
      id: 'x',
      artefact: ArtefactKind.messageReport,
      localPath: 'msg.json',
      createdAt: DateTime.now(),
      status: UploadJobStatus.complete,
    );
    expect(job.canResumeConsent, isFalse);
  });
}
