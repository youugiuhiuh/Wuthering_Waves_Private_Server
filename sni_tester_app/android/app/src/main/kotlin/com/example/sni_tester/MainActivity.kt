package com.example.sni_tester

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private val CHANNEL_FOREGROUND = "com.example.sni_tester/foreground"
    private val CHANNEL_NATIVE = "com.example.sni_tester/native"
    private val CHANNEL_PERMISSION = "com.example.sni_tester/permission"

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL_FOREGROUND).setMethodCallHandler { call, _ ->
            when (call.method) {
                "startForeground" -> {
                    val intent = Intent(this, SniForegroundService::class.java).apply {
                        putExtra("status", "SNI 測試中...")
                    }
                    startServiceCompat(intent)
                }
                "updateProgress" -> {
                    val intent = Intent(this, SniForegroundService::class.java).apply {
                        putExtra("updateOnly", true)
                        putExtra("status", call.argument<String>("status") ?: "SNI 測試中...")
                    }
                    startServiceCompat(intent)
                }
                "stopForeground" -> {
                    val intent = Intent(this, SniForegroundService::class.java).apply {
                        putExtra("finalStatus", call.argument<String>("finalStatus") ?: "測試完成")
                    }
                    startServiceCompat(intent)
                }
            }
        }
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL_NATIVE).setMethodCallHandler { call, result ->
            if (call.method == "getNativeLibDir") {
                result.success(applicationContext.applicationInfo.nativeLibraryDir)
            } else {
                result.notImplemented()
            }
        }
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL_PERMISSION).setMethodCallHandler { call, result ->
            if (call.method == "requestNotification") {
                if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                    ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1001)
                }
                result.success(true)
            }
        }
    }

    private fun startServiceCompat(intent: Intent) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }
}
