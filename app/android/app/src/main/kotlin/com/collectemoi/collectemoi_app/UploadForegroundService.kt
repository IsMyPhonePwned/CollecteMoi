package com.collectemoi.collectemoi_app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

/**
 * Keeps the process alive while CollecteMoi encrypts/uploads an artefact so the
 * user can leave the app (home screen, other apps) without the OS suspending
 * the Dart HTTP upload mid-flight.
 */
class UploadForegroundService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                return START_NOT_STICKY
            }
            else -> {
                val title = intent?.getStringExtra(EXTRA_TITLE)
                    ?: getString(R.string.upload_notif_title_default)
                val text = intent?.getStringExtra(EXTRA_TEXT)
                    ?: getString(R.string.upload_notif_text_default)
                ensureChannel()
                val notification = buildNotification(title, text)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    startForeground(
                        NOTIFICATION_ID,
                        notification,
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
                    )
                } else {
                    @Suppress("DEPRECATION")
                    startForeground(NOTIFICATION_ID, notification)
                }
            }
        }
        return START_STICKY
    }

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val mgr = getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.upload_channel_name),
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = getString(R.string.upload_channel_desc)
            setShowBadge(false)
        }
        mgr.createNotificationChannel(channel)
    }

    private fun buildNotification(title: String, text: String): Notification {
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val pending = PendingIntent.getActivity(
            this,
            0,
            launch,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(pending)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            .build()
    }

    companion object {
        const val CHANNEL_ID = "collectemoi_uploads"
        const val NOTIFICATION_ID = 4201
        const val ACTION_START = "com.collectemoi.UPLOAD_START"
        const val ACTION_STOP = "com.collectemoi.UPLOAD_STOP"
        const val ACTION_UPDATE = "com.collectemoi.UPLOAD_UPDATE"
        const val EXTRA_TITLE = "title"
        const val EXTRA_TEXT = "text"

        fun start(context: Context, title: String, text: String) {
            val intent = Intent(context, UploadForegroundService::class.java).apply {
                action = ACTION_START
                putExtra(EXTRA_TITLE, title)
                putExtra(EXTRA_TEXT, text)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun update(context: Context, title: String, text: String) {
            val intent = Intent(context, UploadForegroundService::class.java).apply {
                action = ACTION_UPDATE
                putExtra(EXTRA_TITLE, title)
                putExtra(EXTRA_TEXT, text)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            val intent = Intent(context, UploadForegroundService::class.java).apply {
                action = ACTION_STOP
            }
            context.startService(intent)
        }

        /** One-shot notification after upload succeeds or fails (replaces the ongoing FG one). */
        fun notifyFinished(context: Context, title: String, text: String, success: Boolean) {
            val mgr = context.getSystemService(NotificationManager::class.java) ?: return
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val channel = NotificationChannel(
                    RESULT_CHANNEL_ID,
                    context.getString(R.string.upload_result_channel_name),
                    NotificationManager.IMPORTANCE_DEFAULT,
                ).apply {
                    description = context.getString(R.string.upload_result_channel_desc)
                    setShowBadge(true)
                }
                mgr.createNotificationChannel(channel)
            }
            val launch = context.packageManager.getLaunchIntentForPackage(context.packageName)
            val pending = PendingIntent.getActivity(
                context,
                1,
                launch,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val icon = if (success) {
                android.R.drawable.stat_sys_upload_done
            } else {
                android.R.drawable.stat_notify_error
            }
            val notification = NotificationCompat.Builder(context, RESULT_CHANNEL_ID)
                .setContentTitle(title)
                .setContentText(text)
                .setSmallIcon(icon)
                .setAutoCancel(true)
                .setOnlyAlertOnce(false)
                .setContentIntent(pending)
                .setCategory(NotificationCompat.CATEGORY_STATUS)
                .setPriority(NotificationCompat.PRIORITY_DEFAULT)
                .build()
            mgr.notify(RESULT_NOTIFICATION_ID, notification)
        }

        const val RESULT_CHANNEL_ID = "collectemoi_upload_results"
        const val RESULT_NOTIFICATION_ID = 4202
    }
}
