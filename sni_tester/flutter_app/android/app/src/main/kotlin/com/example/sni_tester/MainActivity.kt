package com.example.sni_tester

import android.content.Intent
import android.os.Build
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private val CHANNEL = "com.example.sni_tester/foreground"

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL).setMethodCallHandler { call, _ ->
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
    }

    private fun startServiceCompat(intent: Intent) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }
}
