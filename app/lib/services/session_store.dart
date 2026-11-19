import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

import '../models/models.dart';

class SessionStore {
  static const _sessionKey = 'enrollment_session';
  static const _jobsKey = 'upload_jobs';

  Future<EnrollmentSession?> loadSession() async {
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getString(_sessionKey);
    if (raw == null) return null;
    return EnrollmentSession.fromJson(jsonDecode(raw) as Map<String, dynamic>);
  }

  Future<void> saveSession(EnrollmentSession session) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_sessionKey, jsonEncode(session.toJson()));
  }

  Future<void> clearSession() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(_sessionKey);
  }

  Future<List<UploadJob>> loadJobs() async {
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getString(_jobsKey);
    if (raw == null) return [];
    final list = jsonDecode(raw) as List<dynamic>;
    return list
        .map((e) => UploadJob.fromJson(e as Map<String, dynamic>))
        .toList();
  }

  Future<void> saveJobs(List<UploadJob> jobs) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(
      _jobsKey,
      jsonEncode(jobs.map((j) => j.toJson()).toList()),
    );
  }
}
