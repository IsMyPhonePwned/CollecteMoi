import Flutter
import UIKit
import UserNotifications

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  private let appGroupId = "group.com.collectemoi.app"

  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    UNUserNotificationCenter.current().delegate = self
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
      case "notifyUploadFinished":
        let args = call.arguments as? [String: Any]
        let title = args?["title"] as? String ?? "Upload finished"
        let text = args?["text"] as? String ?? ""
        self?.notifyUploadFinished(title: title, body: text) { ok in
          result(ok)
        }
      default:
        result(FlutterMethodNotImplemented)
      }
    }

    let events = FlutterEventChannel(name: "collectemoi/share_events", binaryMessenger: messenger)
    events.setStreamHandler(ShareStreamHandler())
  }

  private func notifyUploadFinished(title: String, body: String, completion: @escaping (Bool) -> Void) {
    let center = UNUserNotificationCenter.current()
    center.requestAuthorization(options: [.alert, .sound, .badge]) { granted, _ in
      guard granted else {
        DispatchQueue.main.async { completion(false) }
        return
      }
      let content = UNMutableNotificationContent()
      content.title = title
      content.body = body
      content.sound = .default
      let request = UNNotificationRequest(
        identifier: "collectemoi.upload.\(UUID().uuidString)",
        content: content,
        trigger: nil
      )
      center.add(request) { error in
        DispatchQueue.main.async { completion(error == nil) }
      }
    }
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
