# iOS Share Extension

Sources and entitlements are ready:

- `ShareViewController.swift` — stream-copy into App Group inbox + `jobs.json`
- `Info.plist` — activation rules for files / images / text
- `ShareExtension.entitlements` — App Group `group.com.collectemoi.app`
- `../Runner/Runner.entitlements` — same App Group on the main app

## Wire the Xcode target (once)

```bash
open ios/Runner.xcworkspace
```

1. **File → New → Target → Share Extension**  
   Product Name: `ShareExtension`  
   Bundle ID: `com.collectemoi.collectemoiApp.ShareExtension`
2. Replace generated Swift / Info.plist with the files in this folder.
3. Signing & Capabilities → **App Groups** → enable `group.com.collectemoi.app` on **Runner** and **ShareExtension**.
4. Set Runner’s Code Signing Entitlements to `Runner/Runner.entitlements` if Xcode did not pick it up.
5. Build & run on a device; share a `sysdiagnose_*.tar.gz` into CollecteMoi.

The main app reads pending jobs via `collectemoi/native` → `pendingShareJobs` (see `Runner/AppDelegate.swift`).
