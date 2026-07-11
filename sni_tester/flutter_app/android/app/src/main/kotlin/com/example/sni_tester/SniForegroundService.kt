package com.example.sni_tester

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

class SniForegroundService : Service() {
    companion object {
        const val CHANNEL_ID = "sni_tester_channel"
        const val NOTIFICATION_ID = 1001
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val updateOnly = intent?.getBooleanExtra("updateOnly", false) ?: false
        val status = intent?.getStringExtra("status") ?: "SNI 測試中..."
        val finalStatus = intent?.getStringExtra("finalStatus")

        if (updateOnly) {
            updateNotification(status)
        } else if (finalStatus != null) {
            showFinalNotification(finalStatus)
            stopForeground(false)
            stopSelf()
        } else {
            startForeground(NOTIFICATION_ID, buildNotification(status))
        }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun updateNotification(status: String) {
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, buildNotification(status))
    }

    private fun showFinalNotification(status: String) {
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("SNI Tester")
            .setContentText(status)
            .setSmallIcon(android.R.drawable.ic_menu_search)
            .setOngoing(false)
            .setSilent(true)
            .build()
        manager.notify(NOTIFICATION_ID, notification)
    }

    private fun buildNotification(status: String): Notification {
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("SNI Tester")
            .setContentText(status)
            .setSmallIcon(android.R.drawable.ic_menu_search)
            .setOngoing(true)
            .setSilent(true)
            .build()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "SNI Tester",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "SNI 測試前台服務"
                setShowBadge(false)
            }
            val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(channel)
        }
    }
}
