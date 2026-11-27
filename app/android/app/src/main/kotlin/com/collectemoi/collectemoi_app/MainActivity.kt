package com.collectemoi.collectemoi_app

import android.Manifest
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
