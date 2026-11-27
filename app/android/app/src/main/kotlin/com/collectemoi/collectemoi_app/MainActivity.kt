package com.collectemoi.collectemoi_app

import android.Manifest
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.EventChannel
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.io.FileOutputStream

class MainActivity : FlutterActivity() {
    private val methodChannelName = "collectemoi/native"
    private val eventChannelName = "collectemoi/share_events"
    private val notifPermissionRequest = 7101

    private var eventSink: EventChannel.EventSink? = null
    private var pendingShare: HashMap<String, Any?>? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, methodChannelName)
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "isDeveloperOptionsEnabled" -> {
                        val enabled = Settings.Global.getInt(
                            contentResolver,
                            Settings.Global.DEVELOPMENT_SETTINGS_ENABLED,
                            0,
                        ) == 1
                        result.success(enabled)
                    }
                    "openDeviceInfoSettings" -> {
                        startActivity(Intent(Settings.ACTION_DEVICE_INFO_SETTINGS))
                        result.success(null)
                    }
                    "openDeveloperSettings" -> {
                        startActivity(Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS))
                        result.success(null)
                    }
                    "openSamsungSysDump" -> {
                        result.success(openSamsungSysDump())
                    }
                    "pendingShareJobs" -> {
                        val list = ArrayList<Map<String, Any?>>()
                        pendingShare?.let { list.add(it) }
                        pendingShare = null
                        result.success(list)
                    }
                    "startUploadForeground" -> {
                        ensureNotificationPermission()
                        val title = call.argument<String>("title")
                            ?: getString(R.string.upload_notif_title_default)
                        val text = call.argument<String>("text")
                            ?: getString(R.string.upload_notif_text_default)
                        UploadForegroundService.start(this, title, text)
                        result.success(true)
                    }
                    "updateUploadForeground" -> {
                        val title = call.argument<String>("title")
                            ?: getString(R.string.upload_notif_title_default)
                        val text = call.argument<String>("text") ?: ""
                        UploadForegroundService.update(this, title, text)
                        result.success(true)
                    }
                    "stopUploadForeground" -> {
                        UploadForegroundService.stop(this)
                        result.success(true)
                    }
                    "notifyUploadFinished" -> {
                        ensureNotificationPermission()
                        val title = call.argument<String>("title")
                            ?: getString(R.string.upload_result_channel_name)
                        val text = call.argument<String>("text") ?: ""
                        val success = call.argument<Boolean>("success") ?: true
                        UploadForegroundService.notifyFinished(this, title, text, success)
                        result.success(true)
                    }
                    else -> result.notImplemented()
                }
            }

        EventChannel(flutterEngine.dartExecutor.binaryMessenger, eventChannelName)
            .setStreamHandler(object : EventChannel.StreamHandler {
                override fun onListen(arguments: Any?, events: EventChannel.EventSink?) {
                    eventSink = events
                    pendingShare?.let {
                        events?.success(it)
                        pendingShare = null
                    }
                }

                override fun onCancel(arguments: Any?) {
                    eventSink = null
                }
            })
    }

    private fun ensureNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        requestPermissions(
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            notifPermissionRequest,
        )
    }

    /**
     * Best-effort open of Samsung SysDump (*#9900#).
     *
     * Direct start of SysDump is usually blocked for third-party apps
     * (signatureOrSystem KEYSTRING). We still try, then fall back to the dialer
     * with the secret code prefilled. Returns: sysdump | dialer | failed.
     */
    private fun openSamsungSysDump(): String {
        val components = listOf(
            ComponentName(
                "com.sec.android.app.servicemodeapp",
                "com.sec.android.app.servicemodeapp.SysDump",
            ),
            ComponentName(
                "com.sec.android.app.servicemodeapp",
                "com.sec.android.app.servicemodeapp.ServiceModeApp",
            ),
        )
        for (component in components) {
            try {
                // Do not rely on resolveActivity — protected components may not
                // resolve for third-party apps even when present.
                startActivity(
                    Intent(Intent.ACTION_MAIN)
                        .setComponent(component)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
                return "sysdump"
            } catch (_: SecurityException) {
                // Expected on modern One UI — SysDump is signature-protected.
            } catch (_: Exception) {
                // Try next candidate / fallback.
            }
        }

        // SECRET_CODE broadcast (same path as dialing *#9900#) — usually also
        // gated, but harmless to attempt.
        try {
            val secret = Intent("android.provider.Telephony.SECRET_CODE").apply {
                data = Uri.parse("android_secret_code://9900")
            }
            sendBroadcast(secret)
        } catch (_: Exception) {
            // ignore
        }

        // Dialer with the code prefilled — user still confirms / finishes dial.
        return if (openDialerWithSecretCode("*#9900#")) "dialer" else "failed"
    }

    private fun openDialerWithSecretCode(code: String): Boolean {
        val encoded = Uri.encode(code)
        val telEncoded = Uri.parse("tel:$encoded")
        val telParts = Uri.fromParts("tel", code, null)
        val dialerPackages = listOf(
            "com.samsung.android.dialer",
            "com.samsung.android.contacts",
            "com.android.phone",
            "com.android.contacts",
            "com.google.android.dialer",
        )

        val candidates = ArrayList<Intent>()
        // Prefer Samsung Phone when present so *#9900# can trigger SysDump.
        for (pkg in dialerPackages) {
            candidates.add(
                Intent(Intent.ACTION_DIAL, telEncoded).setPackage(pkg),
            )
            candidates.add(
                Intent(Intent.ACTION_DIAL).setData(telParts).setPackage(pkg),
            )
        }
        candidates.add(Intent(Intent.ACTION_DIAL, telEncoded))
        candidates.add(Intent(Intent.ACTION_VIEW, telEncoded))
        candidates.add(Intent(Intent.ACTION_DIAL).setData(telParts))

        for (intent in candidates) {
            try {
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                startActivity(intent)
                return true
            } catch (_: Exception) {
                // try next package / URI form
            }
        }
        return false
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handleShareIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleShareIntent(intent)
    }

    private fun handleShareIntent(intent: Intent?) {
        if (intent == null) return
        val action = intent.action ?: return
        if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return

        @Suppress("DEPRECATION")
        val uri: Uri? = when (action) {
            Intent.ACTION_SEND -> intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri
            Intent.ACTION_SEND_MULTIPLE -> {
                val list = intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)
                list?.firstOrNull()
            }
            else -> null
        }
        if (uri == null) return

        try {
            val destDir = File(filesDir, "inbox")
            destDir.mkdirs()
            val dest = File(destDir, "bugreport-${System.currentTimeMillis()}.zip")
            contentResolver.openInputStream(uri)?.use { input ->
                FileOutputStream(dest).use { output ->
                    input.copyTo(output, bufferSize = 256 * 1024)
                }
            } ?: return

            val payload = hashMapOf<String, Any?>(
                "path" to dest.absolutePath,
                "mime" to (intent.type ?: "application/zip"),
                "size" to dest.length(),
            )
            if (eventSink != null) {
                eventSink?.success(payload)
            } else {
                pendingShare = payload
            }
        } catch (e: Exception) {
            // Keep the activity alive; Flutter can surface a retry.
        }
    }
}
