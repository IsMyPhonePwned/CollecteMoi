import 'package:flutter/foundation.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import 'models/models.dart';
import 'screens/collectors/android_bugreport/android_bugreport_flow.dart';
import 'screens/collectors/ios_sysdiagnose/ios_sysdiagnose_flow.dart';
import 'screens/collectors/message_report/message_report_screen.dart';
import 'screens/consent/consent_screen.dart';
import 'screens/enroll/enroll_screen.dart';
import 'screens/home/home_screen.dart';
import 'screens/lab/lab_upload_screen.dart';
import 'screens/my_data/my_data_screen.dart';
import 'screens/settings/settings_screen.dart';
import 'services/upload_queue.dart';

GoRouter createRouter(Listenable refresh) {
  return GoRouter(
    initialLocation: '/',
    refreshListenable: refresh,
    redirect: (context, state) {
      final queue = context.read<UploadQueue>();
      final enrolled = queue.session != null;
      final location = state.matchedLocation;
      final onEnroll = location == '/enroll';
      final onSettings = location == '/settings';
      // Settings stays reachable before enrollment (lab key toggle lives there).
      if (!enrolled && !onEnroll && !onSettings) return '/enroll';
      if (enrolled && onEnroll) return '/';
      // Keep OS-specific collectors off the wrong phone builds.
      if (location == '/collect/android-bugreport' &&
          !ArtefactKind.androidBugreport.availableOnThisDevice) {
        return '/';
      }
      if (location == '/collect/ios-sysdiagnose' &&
          !ArtefactKind.iosSysdiagnose.availableOnThisDevice) {
        return '/';
      }
      if (location == '/lab/upload' &&
          !kIsWeb &&
          (defaultTargetPlatform == TargetPlatform.android ||
              defaultTargetPlatform == TargetPlatform.iOS)) {
        return '/';
      }
      return null;
    },
    routes: [
      GoRoute(path: '/enroll', builder: (_, _) => const EnrollScreen()),
      GoRoute(path: '/settings', builder: (_, _) => const SettingsScreen()),
      GoRoute(path: '/', builder: (_, _) => const HomeScreen()),
      GoRoute(
        path: '/collect/android-bugreport',
        builder: (_, _) => const AndroidBugreportFlow(),
      ),
      GoRoute(
        path: '/collect/ios-sysdiagnose',
        builder: (_, _) => const IosSysdiagnoseFlow(),
      ),
      GoRoute(
        path: '/collect/message',
        builder: (_, _) => const MessageReportScreen(),
      ),
      GoRoute(
        path: '/lab/upload',
        builder: (_, _) => const LabUploadScreen(),
      ),
      GoRoute(
        path: '/consent/:jobId',
        builder: (_, state) =>
            ConsentScreen(jobId: state.pathParameters['jobId']!),
      ),
      GoRoute(path: '/my-data', builder: (_, _) => const MyDataScreen()),
    ],
  );
}

class SessionRefresh extends ChangeNotifier {
  SessionRefresh(UploadQueue queue) {
    queue.addListener(notifyListeners);
  }
}
