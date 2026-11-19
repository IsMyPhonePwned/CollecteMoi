import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  private let appGroupId = "group.com.collectemoi.app"

  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)

    let messenger = engineBridge.applicationRegistrar.messenger()
    let method = FlutterMethodChannel(name: "collectemoi/native", binaryMessenger: messenger)
    method.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "pendingShareJobs":
        result(self?.readPendingJobs() ?? [])
      default:
        result(FlutterMethodNotImplemented)
      }
    }

    let events = FlutterEventChannel(name: "collectemoi/share_events", binaryMessenger: messenger)
    events.setStreamHandler(ShareStreamHandler())
  }

  private func readPendingJobs() -> [[String: Any]] {
    guard let container = FileManager.default.containerURL(
      forSecurityApplicationGroupIdentifier: appGroupId
    ) else { return [] }
    let jobsFile = container.appendingPathComponent("jobs.json")
    guard let data = try? Data(contentsOf: jobsFile),
          let json = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
      return []
    }
    try? FileManager.default.removeItem(at: jobsFile)
    return json
  }
}

private class ShareStreamHandler: NSObject, FlutterStreamHandler {
  func onListen(withArguments arguments: Any?, eventSink events: @escaping FlutterEventSink) -> FlutterError? {
    return nil
  }

  func onCancel(withArguments arguments: Any?) -> FlutterError? {
    return nil
  }
}
