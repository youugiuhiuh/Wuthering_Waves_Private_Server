# SNI Tester Mobile Standalone — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Phone runs `sni_web` independently, results stay on phone, USB transfer to computer.

**Architecture:** Go backend `sni_web` runs as a child process of Flutter on Android. Flutter acts as full controller (file pick → upload → start → SSE progress → export results). Android ForegroundService keeps process alive in background.

**Tech Stack:** Go (sni_web), Flutter/Dart (UI), Kotlin (ForegroundService), Makefile (build)

---

### Task 1: Go Config — add missing fields

**Files:**
- Modify: `sni_tester/pkg/config.go:10-23`

**Interfaces:**
- Produces: Config struct with new Shutdown, GeoProxy fields

**Change:**

Add two fields to `Config` struct:

```go
type Config struct {
    FixedWorkers  int
    ForceRetry    bool
    ResetAll      bool
    TTLDays       int
    MaxLines      int
    Debug         bool
    UseBuiltinDNS bool
    DNSAddr       string
    GeoDBFile     string
    GeoASNFile    string
    BadgerDBDir   string
    OutputDir     string
    Shutdown      bool      // auto-shutdown after completion
    GeoProxy      string    // proxy for GeoIP database download
}
```

- [ ] **Edit `pkg/config.go`: Add `Shutdown bool` and `GeoProxy string` fields**

- [ ] **Verify compilation**: `cd sni_tester && go build ./cmd/sni_web/`
  Wait... this is in my `cmd/sni_tester/` directory... let me check the Makefile structure.

  Run: `go build -o /dev/null ./cmd/sni_web/`

  Expected: no errors (new fields are exported, no unused-variable warnings since they default to zero values)

---

### Task 2: Go handler — extend /api/start param parsing

**Files:**
- Modify: `sni_tester/cmd/sni_web/handlers.go:57-78`

**Interfaces:**
- Consumes: Config struct from Task 1
- Produces: handler that parses all JSON params from Flutter client

**Change:**

Replace the inline params struct in `handleStart` with a full param list matching Flutter's StartParams.toJson():

```go
var params struct {
    MaxConcurrent int    `json:"max_concurrent"`
    DNS           string `json:"dns"`
    TTLDays       int    `json:"ttl_days"`
    ForceRetest   bool   `json:"force_retest"`
    ResetHistory  bool   `json:"reset_history"`
    DebugMode     bool   `json:"debug_mode"`
    MaxLines      int    `json:"max_lines"`
    AutoShutdown  bool   `json:"auto_shutdown"`
    GeoProxy      string `json:"geo_proxy"`
    TimeoutSec    int    `json:"timeout_sec"`
}
```

Apply params to cfg:

```go
cfg := s.cfg
if params.MaxConcurrent > 0 {
    cfg.FixedWorkers = params.MaxConcurrent
}
if params.DNS != "" {
    cfg.DNSAddr = params.DNS
    cfg.UseBuiltinDNS = false
}
if params.TTLDays > 0 {
    cfg.TTLDays = params.TTLDays
}
cfg.ForceRetest = params.ForceRetest
cfg.ResetAll = params.ResetHistory
cfg.Debug = params.DebugMode
if params.MaxLines > 0 {
    cfg.MaxLines = params.MaxLines
}
cfg.Shutdown = params.AutoShutdown
if params.GeoProxy != "" {
    cfg.GeoProxy = params.GeoProxy
}
```

- [ ] **Edit `handlers.go`: replace the inline params struct and apply logic**

- [ ] **Verify compilation**: `go build -o /dev/null ./cmd/sni_web/`

- [ ] **Verify the `/api/status` response has the expected fields (read `handleStatus` - it already returns `running`, `stats` - check it also returns needed info)**

---

### Task 3: Flutter ApiClient — add uploadContent

**Files:**
- Modify: `sni_tester/flutter_app/lib/data/services/api_client.dart`

**Interfaces:**
- Consumes: existing POST /api/upload endpoint
- Produces: `uploadContent(String content)` method

**Change:**

Add `uploadContent` method (for local mode: pass domain content string directly, not a file):

```dart
Future<void> uploadContent(String content) async {
    final request = http.MultipartRequest(
      'POST',
      Uri.parse('$baseUrl/api/upload'),
    );
    request.files.add(
      http.MultipartFile.fromString('file', content, filename: 'domains.txt'),
    );
    final streamed = await request.send().timeout(const Duration(seconds: 30));
    if (streamed.statusCode != 200) {
      final body = await streamed.stream.bytesToString();
      throw ApiException('Upload failed: ${streamed.statusCode} $body');
    }
}
```

- [ ] **Edit `api_client.dart`: add `uploadContent(String content)` method**

- [ ] **Verify with Flutter analyzer**: `cd flutter_app && flutter analyze --no-pub`

---

### Task 4: Flutter ViewModel — fix startTest, add exportResults

**Files:**
- Modify: `sni_tester/flutter_app/lib/ui/features/home/view_models/home_view_model.dart`

**Interfaces:**
- Consumes: `api.uploadContent()`, `api.downloadResult()`
- Produces: fixed `startTest()` flow, new `exportResults()` method

**Change:**

**a) Fix `startTest()` — upload file content before starting:**

```dart
Future<void> startTest(StartParams params) async {
    try {
      // Read and upload domain file content to backend
      final file = File(params.domainsFile);
      if (await file.exists()) {
        final content = await file.readAsString();
        await api.uploadContent(content);
      }
      await api.startTest(params);
      _running = true;
      _results = [];
      _downloadPath = null;
      _error = null;
      notifyListeners();
      _connectSSE();
    } catch (e) {
      _error = 'Start failed: $e';
      notifyListeners();
    }
}
```

**b) Add `exportResults()` method:**

```dart
Future<void> exportResults() async {
    _downloadLoading = true;
    notifyListeners();
    try {
      final data = await api.downloadResult();
      final dir = Directory('/storage/emulated/0/Download');
      if (!await dir.exists()) {
        await dir.create(recursive: true);
      }
      final ts = DateTime.now().millisecondsSinceEpoch;
      final path = '${dir.path}/sni_results_$ts.zip';
      await File(path).writeAsBytes(data);
      _downloadPath = path;
    } catch (e) {
      _error = 'Export failed: $e';
    }
    _downloadLoading = false;
    notifyListeners();
}
```

- [ ] **Edit `home_view_model.dart`: modify `startTest()`, add `exportResults()`**

- [ ] **Flutter analyze check**: `cd flutter_app && flutter analyze --no-pub`

---

### Task 5: Flutter HomeScreen — show export button on mobile

**Files:**
- Modify: `sni_tester/flutter_app/lib/ui/features/home/views/home_screen.dart`

**Interfaces:**
- Consumes: `_vm.exportResults()`, `_vm.downloadPath`, `_vm.downloadLoading`

**Change:**

Currently the ResultDownloadCard is wrapped in `if (isDesktop)` and `if (!_vm.running && _vm.results.isNotEmpty)`. Remove the `isDesktop` check so it also shows on mobile:

```dart
if (!_vm.running && _vm.results.isNotEmpty)
    ResultDownloadCard(
      loading: _vm.downloadLoading,
      progress: _vm.downloadProgress,
      savedPath: _vm.downloadPath,
      onDownload: _vm.exportResults,
      onOpenFolder: () {},
    ),
```

- [ ] **Edit `home_screen.dart`: remove `isDesktop` guard from ResultDownloadCard**

- [ ] **Flutter analyze check**: `cd flutter_app && flutter analyze --no-pub`

---

### Task 6: Android ForegroundService — manifest + service

**Files:**
- Modify: `sni_tester/flutter_app/android/app/src/main/AndroidManifest.xml`
- Create: `sni_tester/flutter_app/android/app/src/main/kotlin/com/example/sni_tester/SniForegroundService.kt`
- Modify: `sni_tester/flutter_app/android/app/src/main/kotlin/com/example/sni_tester/MainActivity.kt`

**Interfaces:**
- Produces: MethodChannel `com.example.sni_tester/foreground` with commands `startForeground`, `updateProgress`, `stopForeground`

**a) AndroidManifest.xml — add permissions and service declaration:**

```xml
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE_SPECIAL_USE" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
    <application ...>
        ...
        <service
            android:name=".SniForegroundService"
            android:foregroundServiceType="specialUse"
            android:exported="false" />
    </application>
</manifest>
```

**b) SniForegroundService.kt — create foreground service:**

```kotlin
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
        var currentStatus: String = "SNI 測試中..."
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notification = buildNotification(currentStatus)
        startForeground(NOTIFICATION_ID, notification)
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    fun updateNotification(status: String) {
        currentStatus = status
        val notification = buildNotification(status)
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, notification)
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

    private fun buildNotification(status: String): Notification {
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("SNI Tester")
            .setContentText(status)
            .setSmallIcon(android.R.drawable.ic_menu_search)
            .setOngoing(true)
            .setSilent(true)
            .build()
    }
}
```

**c) MainActivity.kt — add MethodChannel:**

```kotlin
package com.example.sni_tester

import android.content.Intent
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private val CHANNEL = "com.example.sni_tester/foreground"
    private var foregroundService: SniForegroundService? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL).setMethodCallHandler { call, _ ->
            when (call.method) {
                "startForeground" -> {
                    val intent = Intent(this, SniForegroundService::class.java)
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        startForegroundService(intent)
                    } else {
                        startService(intent)
                    }
                }
                "updateProgress" -> {
                    val status = call.argument<String>("status") ?: "SNI 測試中..."
                    // Update via a bound service or broadcast
                }
                "stopForeground" -> {
                    val intent = Intent(this, SniForegroundService::class.java)
                    stopService(intent)
                }
            }
        }
    }
}
```

Wait, updating the notification from a non-bound service requires a different approach. Let me use a simpler pattern: use a companion object holder on the service itself, and update via `stopService → startService` with intent extras.

Better approach — use Intent extras:

```kotlin
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
                    startForegroundServiceCompat(intent)
                }
                "updateProgress" -> {
                    val intent = Intent(this, SniForegroundService::class.java).apply {
                        putExtra("status", call.argument<String>("status") ?: "SNI 測試中...")
                        putExtra("updateOnly", true)
                    }
                    startForegroundServiceCompat(intent)
                }
                "stopForeground" -> {
                    val intent = Intent(this, SniForegroundService::class.java).apply {
                        putExtra("finalStatus", call.argument<String>("finalStatus") ?: "測試完成")
                    }
                    startForegroundServiceCompat(intent)
                }
            }
        }
    }

    private fun startForegroundServiceCompat(intent: Intent) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }
}
```

And SniForegroundService handles updateOnly/finalStatus:

```kotlin
class SniForegroundService : Service() {
    companion object {
        const val CHANNEL_ID = "sni_tester_channel"
        const val NOTIFICATION_ID = 1001
    }

    override fun onCreate() { ... } // same as above, create channel

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

    private fun buildNotification(status: String): Notification { ... } // same as above
    private fun createNotificationChannel() { ... } // same as above
}
```

- [ ] **Edit `AndroidManifest.xml`: add permissions + service declaration**

- [ ] **Create `SniForegroundService.kt`**

- [ ] **Replace `MainActivity.kt` content with MethodChannel version**

- [ ] **Verify compilation**: `cd flutter_app && flutter build apk --debug --no-pub` (just check it compiles, don't wait for full build)

  Alternative: `cd flutter_app && flutter analyze --no-pub`

---

### Task 7: Flutter NotificationService — integrate foreground service

**Files:**
- Create: `sni_tester/flutter_app/lib/data/services/notification_service.dart`
- Modify: `sni_tester/flutter_app/lib/ui/features/home/view_models/home_view_model.dart`

**Interfaces:**
- Consumes: MethodChannel `com.example.sni_tester/foreground`
- Produces: Dart APIs that ViewModel calls during test lifecycle

**a) Create notification_service.dart:**

```dart
import 'package:flutter/services.dart';

class NotificationService {
  static const _channel = MethodChannel('com.example.sni_tester/foreground');

  static Future<void> start() async {
    try {
      await _channel.invokeMethod('startForeground');
    } catch (_) {}
  }

  static Future<void> updateProgress(int done, int total) async {
    try {
      final status = '已完成 $done/$total (${total > 0 ? (done * 100 ~/ total) : 0}%)';
      await _channel.invokeMethod('updateProgress', {'status': status});
    } catch (_) {}
  }

  static Future<void> complete(int success, int failed) async {
    try {
      final status = '完成: $success 成功, $failed 失败';
      await _channel.invokeMethod('stopForeground', {'finalStatus': status});
    } catch (_) {}
  }
}
```

**b) Integrate into ViewModel:**

In `startTest()`, after `notifyListeners()`:
```dart
NotificationService.start();
```

In `_connectSSE()` stream listener, add:
```dart
NotificationService.updateProgress(_stats.success + _stats.failed, _stats.total);
```

When SSE done (`_stats.done >= _stats.total`):
```dart
NotificationService.complete(_stats.success, _stats.failed);
```

- [ ] **Create `notification_service.dart`**

- [ ] **Edit `home_view_model.dart`: import, add start/update/complete calls**

- [ ] **Flutter analyze**: `cd flutter_app && flutter analyze --no-pub`

---

### Task 8: Build & verification

**Files:**
- Modify: `sni_tester/Makefile:22-25` (verify phone-pull target)

**Interfaces:**
- Consumes: all previous tasks
- Produces: release APK

**Change:**

Verify `phone-pull` target pulls to the right directory:

```makefile
phone-pull:
    mkdir -p ../rust/aegis/src/resources/sni/
    adb pull /data/local/tmp/$(OUTPUT_DIR)/. ../rust/aegis/src/resources/sni/
    @echo "Results pulled to ../rust/aegis/src/resources/sni/"
```

- [ ] **Run full Go build**: `cd sni_tester && GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -o /dev/null ./cmd/sni_web/`

- [ ] **Bundle Go asset**: `cd sni_tester && GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -o flutter_app/assets/sni_web ./cmd/sni_web/`

- [ ] **Build APK debug**: `cd sni_tester/flutter_app && flutter build apk --debug`

- [ ] **Verify phone-pull** in Makefile is correct

---
