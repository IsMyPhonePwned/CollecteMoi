import UIKit
import UniformTypeIdentifiers
import MobileCoreServices

/// Minimal share extension: stream-copy sysdiagnose / message payloads into the App Group.
/// Wire this target in Xcode with App Group `group.com.collectemoi.app`.
class ShareViewController: UIViewController {
    private let appGroupId = "group.com.collectemoi.app"

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        Task { await handleIncoming() }
    }

    private func handleIncoming() async {
        guard let item = extensionContext?.inputItems.first as? NSExtensionItem,
              let providers = item.attachments else {
            close()
            return
        }

        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.gnuZipArchive.identifier) ||
                provider.hasItemConformingToTypeIdentifier(UTType.data.identifier) {
                do {
                    let url = try await loadFile(provider: provider)
                    if let url {
                        try copyIntoAppGroup(url)
                    }
                } catch {
                    // Finish gracefully; main app can offer Files import fallback.
                }
            }
        }
        close()
    }

    private func loadFile(provider: NSItemProvider) async throws -> URL? {
        try await withCheckedThrowingContinuation { cont in
            let typeId: String
            if provider.hasItemConformingToTypeIdentifier(UTType.gnuZipArchive.identifier) {
                typeId = UTType.gnuZipArchive.identifier
            } else {
                typeId = UTType.data.identifier
            }
            provider.loadFileRepresentation(forTypeIdentifier: typeId) { url, error in
                if let error {
                    cont.resume(throwing: error)
                } else {
                    cont.resume(returning: url)
                }
            }
        }
    }

    private func copyIntoAppGroup(_ source: URL) throws {
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupId
        ) else { return }

        let inbox = container.appendingPathComponent("inbox", isDirectory: true)
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)

        let name = source.lastPathComponent
        let dest = inbox.appendingPathComponent("\(Int(Date().timeIntervalSince1970))-\(name)")

        // Stream copy via coordinator to keep memory low for >200 MB archives.
        var coordinatorError: NSError?
        let coordinator = NSFileCoordinator()
        coordinator.coordinate(readingItemAt: source, options: [], error: &coordinatorError) { readable in
            try? FileManager.default.copyItem(at: readable, to: dest)
        }
        if let coordinatorError { throw coordinatorError }

        let jobs = container.appendingPathComponent("jobs.json")
        var list: [[String: Any]] = []
        if let data = try? Data(contentsOf: jobs),
           let json = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] {
            list = json
        }
        list.append([
            "path": dest.path,
            "name": name,
            "created_at": ISO8601DateFormatter().string(from: Date()),
        ])
        let out = try JSONSerialization.data(withJSONObject: list, options: [.prettyPrinted])
        try out.write(to: jobs, options: .atomic)
    }

    private func close() {
        extensionContext?.completeRequest(returningItems: nil, completionHandler: nil)
    }
}
