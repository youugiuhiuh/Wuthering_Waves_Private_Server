# Desktop-Mobile Collaboration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform the single-device Flutter app into desktop controller + mobile executor with ADB/WiFi collaboration.

**Architecture:** Single Flutter project with runtime platform detection (`dart:io Platform`). Desktop gets full control panel (ConnectionCard, RemoteFileCard, AdvancedSettingsCard, ResultDownloadCard). Phone gets AppModeCard (local/controlled toggle) + existing widgets. Go backend gets 2 new endpoints (`/api/files`, `DELETE /api/files`).

**Tech Stack:** Flutter 3.x + shared_preferences + existing Go backend (extended).

---

### File Map

| Responsibility | File |
|---|---|
| Extended models | `lib/data/models/models.dart` |
| Preferences persistence | `lib/data/services/preferences_service.dart` (NEW) |
| Platform detection | `lib/ui/core/platform.dart` (NEW) |
| Remote API methods | `lib/data/services/api_client.dart` |
| Connection panel | `lib/ui/features/home/views/widgets/connection_card.dart` (NEW) |
| Phone mode switch | `lib/ui/features/home/views/widgets/app_mode_card.dart` (NEW) |
| Remote file mgmt | `lib/ui/features/home/views/widgets/remote_file_card.dart` (NEW) |
| Extended basic settings | `lib/ui/features/home/views/widgets/settings_card.dart` |
| Full advanced settings | `lib/ui/features/home/views/widgets/advanced_settings_card.dart` (NEW) |
| Result download | `lib/ui/features/home/views/widgets/result_download_card.dart` (NEW) |
| State machine | `lib/ui/features/home/view_models/home_view_model.dart` |
| Screen assembly | `lib/ui/features/home/views/home_screen.dart` |
| App entry | `lib/main.dart` |
| Go API handlers | `cmd/sni_web/handlers.go` |
| Go routes | `cmd/sni_web/main.go` |

---

### Task 1: Extend data models

**Files:**
- Modify: `lib/data/models/models.dart`

- [ ] **Add FileEntry, ConnectionState, ConnMode, RemoteConfig enums/models**

Open `lib/data/models/models.dart` and add at the bottom:

```dart
enum ConnMode { local, usb, wifi }

enum ConnectionState { disconnected, connecting, connected, error }

class FileEntry {
  final String name;
  final int size;
  final String modTime;

  const FileEntry({
    required this.name,
    required this.size,
    required this.modTime,
  });

  factory FileEntry.fromJson(Map<String, dynamic> json) {
    return FileEntry(
      name: json['name'] as String? ?? '',
      size: json['size'] as int? ?? 0,
      modTime: json['mod_time'] as String? ?? '',
    );
  }

  String get sizeFormatted {
    if (size < 1024) return '$size B';
    if (size < 1024 * 1024) return '${(size / 1024).toStringAsFixed(1)} KB';
    return '${(size / (1024 * 1024)).toStringAsFixed(1)} MB';
  }
}

class RemoteConfig {
  final ConnMode mode;
  final String host;
  final int port;

  const RemoteConfig({
    this.mode = ConnMode.usb,
    this.host = 'localhost',
    this.port = 18080,
  });

  String get baseUrl => 'http://$host:$port';
}
```

Then extend `StartParams` in the same file — add fields after existing ones:

```dart
  final String? dns;
  final int? ttlDays;
  final bool? forceRetest;
  final bool? debugMode;
  final bool? resetHistory;
  final String? geoProxy;
  final int? maxLines;
  final bool? autoShutdown;

  const StartParams({
    required this.serversFile,
    required this.domainsFile,
    this.timeoutSec = 5,
    this.maxConcurrent = 20,
    this.dns,
    this.ttlDays,
    this.forceRetest,
    this.debugMode,
    this.resetHistory,
    this.geoProxy,
    this.maxLines,
    this.autoShutdown,
  });
```

And update `toJson()`:

```dart
  Map<String, dynamic> toJson() => {
        'servers_file': serversFile,
        'domains_file': domainsFile,
        'timeout_sec': timeoutSec,
        'max_concurrent': maxConcurrent,
        if (dns != null) 'dns': dns,
        if (ttlDays != null) 'ttl_days': ttlDays,
        if (forceRetest != null) 'force_retest': forceRetest,
        if (debugMode != null) 'debug_mode': debugMode,
        if (resetHistory != null) 'reset_history': resetHistory,
        if (geoProxy != null) 'geo_proxy': geoProxy,
        if (maxLines != null) 'max_lines': maxLines,
        if (autoShutdown != null) 'auto_shutdown': autoShutdown,
      };
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/data/models/models.dart
git commit -m "feat(models): add FileEntry, ConnMode, RemoteConfig, extended StartParams"
```

---

### Task 2: Create PreferencesService

**Files:**
- Create: `lib/data/services/preferences_service.dart`

- [ ] **Write PreferencesService**

```dart
import 'package:shared_preferences/shared_preferences.dart';

import '../models/models.dart';

class PreferencesService {
  static const _kMode = 'conn_mode';
  static const _kHost = 'conn_host';
  static const _kPort = 'conn_port';
  static const _kTimeout = 'timeout_sec';
  static const _kConcurrent = 'max_concurrent';
  static const _kDns = 'dns';
  static const _kCustomDns = 'custom_dns';
  static const _kTtl = 'ttl_days';
  static const _kForce = 'force_retest';
  static const _kDebug = 'debug_mode';
  static const _kProxy = 'geo_proxy';
  static const _kMaxLines = 'max_lines';
  static const _kAutoShutdown = 'auto_shutdown';
  static const _kAimd = 'aimd_enabled';
  static const _kFixedWorkers = 'fixed_workers';

  final SharedPreferences _prefs;

  PreferencesService(this._prefs);

  RemoteConfig get remoteConfig {
    final modeStr = _prefs.getString(_kMode) ?? 'usb';
    return RemoteConfig(
      mode: ConnMode.values.firstWhere((e) => e.name == modeStr,
          orElse: () => ConnMode.usb),
      host: _prefs.getString(_kHost) ?? 'localhost',
      port: _prefs.getInt(_kPort) ?? 18080,
    );
  }

  Future<void> saveRemoteConfig(RemoteConfig cfg) async {
    await _prefs.setString(_kMode, cfg.mode.name);
    await _prefs.setString(_kHost, cfg.host);
    await _prefs.setInt(_kPort, cfg.port);
  }

  int get timeoutSec => _prefs.getInt(_kTimeout) ?? 5;
  int get maxConcurrent => _prefs.getInt(_kConcurrent) ?? 20;
  String? get dns => _prefs.getString(_kDns);
  String? get customDns => _prefs.getString(_kCustomDns);
  int get ttlDays => _prefs.getInt(_kTtl) ?? 7;
  bool get forceRetest => _prefs.getBool(_kForce) ?? false;
  bool get debugMode => _prefs.getBool(_kDebug) ?? false;
  String? get geoProxy => _prefs.getString(_kProxy);
  int get maxLines => _prefs.getInt(_kMaxLines) ?? 0;
  bool get autoShutdown => _prefs.getBool(_kAutoShutdown) ?? false;
  bool get aimdEnabled => _prefs.getBool(_kAimd) ?? true;
  int get fixedWorkers => _prefs.getInt(_kFixedWorkers) ?? 20;

  Future<void> saveAll({
    int? timeoutSec,
    int? maxConcurrent,
    String? dns,
    String? customDns,
    int? ttlDays,
    bool? forceRetest,
    bool? debugMode,
    String? geoProxy,
    int? maxLines,
    bool? autoShutdown,
    bool? aimdEnabled,
    int? fixedWorkers,
  }) async {
    final batch = <String, dynamic>{};
    if (timeoutSec != null) batch[_kTimeout] = timeoutSec;
    if (maxConcurrent != null) batch[_kConcurrent] = maxConcurrent;
    if (dns != null) batch[_kDns] = dns;
    if (customDns != null) batch[_kCustomDns] = customDns;
    if (ttlDays != null) batch[_kTtl] = ttlDays;
    if (forceRetest != null) batch[_kForce] = forceRetest;
    if (debugMode != null) batch[_kDebug] = debugMode;
    if (geoProxy != null) batch[_kProxy] = geoProxy;
    if (maxLines != null) batch[_kMaxLines] = maxLines;
    if (autoShutdown != null) batch[_kAutoShutdown] = autoShutdown;
    if (aimdEnabled != null) batch[_kAimd] = aimdEnabled;
    if (fixedWorkers != null) batch[_kFixedWorkers] = fixedWorkers;
    for (final e in batch.entries) {
      if (e.value is int) {
        await _prefs.setInt(e.key, e.value as int);
      } else if (e.value is String) {
        await _prefs.setString(e.key, e.value as String);
      } else if (e.value is bool) {
        await _prefs.setBool(e.key, e.value as bool);
      }
    }
  }
}
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors on this file

- [ ] **Commit**

```bash
git add flutter_app/lib/data/services/preferences_service.dart
git commit -m "feat: add PreferencesService with SharedPreferences persistence"
```

---

### Task 3: Add platform detection helper

**Files:**
- Create: `lib/ui/core/platform.dart`

- [ ] **Write platform helpers**

```dart
import 'dart:io' show Platform;

bool get isDesktop =>
    Platform.isLinux || Platform.isWindows || Platform.isMacOS;

bool get isMobile => Platform.isAndroid || Platform.isIOS;
```

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/core/platform.dart
git commit -m "feat: add platform detection helpers"
```

---

### Task 4: Add Go backend endpoints

**Files:**
- Modify: `cmd/sni_web/handlers.go`
- Modify: `cmd/sni_web/main.go`

- [ ] **Add HandleListFiles and HandleDeleteFile to handlers.go**

Open `cmd/sni_web/handlers.go`, add after existing handlers:

```go
func (s *Server) HandleListFiles(w http.ResponseWriter, r *http.Request) {
	entries, err := os.ReadDir(s.cfg.OutputDir)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	type fileInfo struct {
		Name    string `json:"name"`
		Size    int64  `json:"size"`
		ModTime string `json:"mod_time"`
	}
	var files []fileInfo
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		files = append(files, fileInfo{
			Name:    e.Name(),
			Size:    info.Size(),
			ModTime: info.ModTime().UTC().Format(time.RFC3339),
		})
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(files)
}

func (s *Server) HandleDeleteFile(w http.ResponseWriter, r *http.Request) {
	name := r.URL.Query().Get("name")
	if name == "" {
		http.Error(w, "name query param required", http.StatusBadRequest)
		return
	}
	path := filepath.Join(s.cfg.OutputDir, name)
	if err := os.Remove(path); err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	w.WriteHeader(http.StatusOK)
	json.NewEncoder(w).Encode(map[string]string{"status": "deleted"})
}
```

- [ ] **Register routes in main.go**

Open `cmd/sni_web/main.go`, add after existing routes:

```go
	mux.HandleFunc("GET /api/files", srv.HandleListFiles)
	mux.HandleFunc("DELETE /api/files", srv.HandleDeleteFile)
```

Also add `"path/filepath"` and `"time"` to imports.

- [ ] **Build and verify**

Run: `go build ./cmd/sni_web/`
Expected: BUILD OK

- [ ] **Commit**

```bash
git add sni_tester/cmd/sni_web/
git commit -m "feat(api): add /api/files list + delete endpoints"
```

---

### Task 5: Extend API Client with remote methods

**Files:**
- Modify: `lib/data/services/api_client.dart`

- [ ] **Add uploadFile, listFiles, deleteFile, downloadResult, connectToRemote**

Open `lib/data/services/api_client.dart`. Add after `progressStream()`:
Note: add `dart:io` and `'package:http/http.dart'` are already imported.

```dart
  Future<void> connectToRemote(String url) async {
    final healthUrl = Uri.parse('$url/api/health');
    final res = await _http.get(healthUrl).timeout(const Duration(seconds: 5));
    if (res.statusCode != 200) {
      throw ApiException('Health check failed: ${res.statusCode}');
    }
  }

  Future<List<FileEntry>> listFiles() async {
    final res = await _http
        .get(Uri.parse('$baseUrl/api/files'))
        .timeout(const Duration(seconds: 5));
    if (res.statusCode != 200) {
      throw ApiException('List files failed: ${res.statusCode}');
    }
    final list = jsonDecode(res.body) as List<dynamic>;
    return list
        .map((e) => FileEntry.fromJson(e as Map<String, dynamic>))
        .toList();
  }

  Future<void> deleteFile(String name) async {
    final uri = Uri.parse('$baseUrl/api/files?name=${Uri.encodeComponent(name)}');
    final res = await _http.delete(uri).timeout(const Duration(seconds: 5));
    if (res.statusCode != 200) {
      throw ApiException('Delete failed: ${res.statusCode}');
    }
  }

  Future<void> uploadFile(String localPath, String remoteName) async {
    final file = File(localPath);
    if (!file.existsSync()) {
      throw ApiException('File not found: $localPath');
    }
    final request = http.MultipartRequest(
      'POST',
      Uri.parse('$baseUrl/api/upload'),
    );
    request.files.add(
      await http.MultipartFile.fromPath('file', localPath, filename: remoteName),
    );
    final streamed = await request.send().timeout(const Duration(seconds: 30));
    if (streamed.statusCode != 200) {
      final body = await streamed.stream.bytesToString();
      throw ApiException('Upload failed: ${streamed.statusCode} $body');
    }
  }

  Future<List<int>> downloadResult() async {
    final res = await _http
        .get(Uri.parse('$baseUrl/api/download'))
        .timeout(const Duration(seconds: 30));
    if (res.statusCode != 200) {
      throw ApiException('Download failed: ${res.statusCode}');
    }
    return res.bodyBytes.toList();
  }
```

Also add the import for `FileEntry` at the top:

```dart
import '../models/models.dart';
```

(Already present, verify it's there.)

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/data/services/api_client.dart
git commit -m "feat(api): add remote methods - connect, upload, list, delete, download"
```

---

### Task 6: Create AppModeCard (Phone mode switch)

**Files:**
- Create: `lib/ui/features/home/views/widgets/app_mode_card.dart`

- [ ] **Write AppModeCard widget**

```dart
import 'package:flutter/material.dart';

enum PhoneMode { local, controlled }

class AppModeCard extends StatelessWidget {
  final PhoneMode mode;
  final ValueChanged<PhoneMode> onModeChanged;
  final bool backendRunning;

  const AppModeCard({
    super.key,
    required this.mode,
    required this.onModeChanged,
    this.backendRunning = false,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Mode', style: theme.textTheme.titleMedium),
            const SizedBox(height: 12),
            SegmentedButton<PhoneMode>(
              segments: const [
                ButtonSegment(
                  value: PhoneMode.local,
                  label: Text('Local'),
                  icon: Icon(Icons.phone_android),
                ),
                ButtonSegment(
                  value: PhoneMode.controlled,
                  label: Text('Controlled'),
                  icon: Icon(Icons.cast),
                ),
              ],
              selected: {mode},
              onSelectionChanged: (s) => onModeChanged(s.first),
            ),
            if (mode == PhoneMode.controlled) ...[
              const SizedBox(height: 16),
              Row(
                children: [
                  Icon(Icons.wifi, color: backendRunning ? Colors.green : theme.colorScheme.outline),
                  const SizedBox(width: 8),
                  Text(
                    backendRunning ? 'Port 18080 — Waiting for desktop...' : 'Backend not running',
                    style: TextStyle(
                      color: backendRunning ? null : theme.colorScheme.error,
                    ),
                  ),
                ],
              ),
            ],
          ],
        ),
      ),
    );
  }
}
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/features/home/views/widgets/app_mode_card.dart
git commit -m "feat: add AppModeCard for phone local/controlled mode switch"
```

---

### Task 7: Create ConnectionCard (Desktop connection panel)

**Files:**
- Create: `lib/ui/features/home/views/widgets/connection_card.dart`

- [ ] **Write ConnectionCard widget**

```dart
import 'package:flutter/material.dart';

import '../../../../data/models/models.dart';

class ConnectionCard extends StatelessWidget {
  final ConnMode mode;
  final ConnectionState connState;
  final String? deviceName;
  final String? errorMsg;
  final String host;
  final int port;
  final ValueChanged<ConnMode> onModeChanged;
  final VoidCallback onConnect;
  final VoidCallback onDisconnect;
  final ValueChanged<String> onHostChanged;
  final ValueChanged<int> onPortChanged;

  const ConnectionCard({
    super.key,
    required this.mode,
    required this.connState,
    this.deviceName,
    this.errorMsg,
    this.host = 'localhost',
    this.port = 18080,
    required this.onModeChanged,
    required this.onConnect,
    required this.onDisconnect,
    required this.onHostChanged,
    required this.onPortChanged,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isConnected = connState == ConnectionState.connected;
    final isConnecting = connState == ConnectionState.connecting;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Connection', style: theme.textTheme.titleMedium),
            const SizedBox(height: 12),
            SegmentedButton<ConnMode>(
              segments: const [
                ButtonSegment(value: ConnMode.local, label: Text('Local'), icon: Icon(Icons.computer)),
                ButtonSegment(value: ConnMode.usb, label: Text('USB'), icon: Icon(Icons.usb)),
                ButtonSegment(value: ConnMode.wifi, label: Text('WiFi'), icon: Icon(Icons.wifi)),
              ],
              selected: {mode},
              onSelectionChanged: (s) => onModeChanged(s.first),
            ),
            if (mode == ConnMode.wifi) ...[
              const SizedBox(height: 12),
              Row(
                children: [
                  Expanded(
                    child: TextField(
                      decoration: const InputDecoration(
                        labelText: 'Host',
                        isDense: true,
                        hintText: '192.168.1.100',
                      ),
                      controller: TextEditingController(text: host),
                      onChanged: onHostChanged,
                    ),
                  ),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 80,
                    child: TextField(
                      decoration: const InputDecoration(
                        labelText: 'Port',
                        isDense: true,
                      ),
                      controller: TextEditingController(text: port.toString()),
                      keyboardType: TextInputType.number,
                      onChanged: (v) {
                        final n = int.tryParse(v);
                        if (n != null) onPortChanged(n);
                      },
                    ),
                  ),
                ],
              ),
            ],
            const SizedBox(height: 12),
            Row(
              children: [
                _statusIndicator(context),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    isConnected
                        ? 'Connected ${deviceName != null ? 'to $deviceName' : ''}'
                        : isConnecting
                            ? 'Connecting...'
                            : errorMsg ?? 'Not connected',
                    style: TextStyle(
                      color: isConnected
                          ? Colors.green
                          : isConnecting
                              ? theme.colorScheme.primary
                              : errorMsg != null
                                  ? theme.colorScheme.error
                                  : theme.colorScheme.outline,
                    ),
                  ),
                ),
                if (isConnected)
                  TextButton(onPressed: onDisconnect, child: const Text('Disconnect'))
                else
                  FilledButton.tonal(
                    onPressed: isConnecting ? null : onConnect,
                    child: Text(isConnecting ? '...' : 'Connect'),
                  ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  Widget _statusIndicator(BuildContext context) {
    final color = switch (connState) {
      ConnectionState.connected => Colors.green,
      ConnectionState.connecting => Theme.of(context).colorScheme.primary,
      ConnectionState.error => Theme.of(context).colorScheme.error,
      ConnectionState.disconnected => Theme.of(context).colorScheme.outline,
    };
    return Container(
      width: 12,
      height: 12,
      decoration: BoxDecoration(
        color: color,
        shape: BoxShape.circle,
      ),
    );
  }
}
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/features/home/views/widgets/connection_card.dart
git commit -m "feat: add ConnectionCard with USB/WiFi/Local mode and status"
```

---

### Task 8: Create RemoteFileCard

**Files:**
- Create: `lib/ui/features/home/views/widgets/remote_file_card.dart`

- [ ] **Write RemoteFileCard widget**

```dart
import 'package:flutter/material.dart';

import '../../../../data/models/models.dart';

class RemoteFileCard extends StatelessWidget {
  final List<FileEntry> files;
  final bool loading;
  final VoidCallback onRefresh;
  final VoidCallback onUpload;
  final ValueChanged<String> onDelete;
  final double? uploadProgress;

  const RemoteFileCard({
    super.key,
    required this.files,
    this.loading = false,
    required this.onRefresh,
    required this.onUpload,
    required this.onDelete,
    this.uploadProgress,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Text('Phone Files', style: theme.textTheme.titleMedium),
                const Spacer(),
                if (loading)
                  const SizedBox(
                    width: 16, height: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                else
                  IconButton(
                    icon: const Icon(Icons.refresh, size: 18),
                    onPressed: onRefresh,
                    tooltip: 'Refresh',
                  ),
                IconButton(
                  icon: const Icon(Icons.cloud_upload, size: 18),
                  onPressed: onUpload,
                  tooltip: 'Upload file',
                ),
              ],
            ),
            if (uploadProgress != null) ...[
              const SizedBox(height: 8),
              LinearProgressIndicator(value: uploadProgress),
              const SizedBox(height: 4),
              Text('Uploading... ${(uploadProgress! * 100).toStringAsFixed(0)}%',
                  style: const TextStyle(fontSize: 12)),
            ],
            const SizedBox(height: 8),
            if (files.isEmpty && !loading)
              Text('No files on phone', style: TextStyle(color: theme.colorScheme.outline))
            else
              ...files.map((f) => ListTile(
                    dense: true,
                    leading: const Icon(Icons.description, size: 20),
                    title: Text(f.name, style: const TextStyle(fontSize: 13)),
                    trailing: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Text(f.sizeFormatted,
                            style: TextStyle(fontSize: 12, color: theme.colorScheme.outline)),
                        const SizedBox(width: 8),
                        InkWell(
                          onTap: () => onDelete(f.name),
                          child: Icon(Icons.delete_outline, size: 18,
                              color: theme.colorScheme.error),
                        ),
                      ],
                    ),
                  )),
          ],
        ),
      ),
    );
  }
}
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/features/home/views/widgets/remote_file_card.dart
git commit -m "feat: add RemoteFileCard for phone file list, upload, delete"
```

---

### Task 9: Extend SettingsCard with DNS, debug, force, reset

**Files:**
- Modify: `lib/ui/features/home/views/widgets/settings_card.dart`

- [ ] **Replace SettingsCard with extended version**

Replace entire content with:

```dart
import 'package:flutter/material.dart';

class SettingsCard extends StatelessWidget {
  final int timeoutSec;
  final int maxConcurrent;
  final String? dns;
  final bool debugMode;
  final bool forceRetest;
  final ValueChanged<int> onTimeoutChanged;
  final ValueChanged<int> onConcurrentChanged;
  final ValueChanged<String?> onDnsChanged;
  final ValueChanged<bool> onDebugChanged;
  final ValueChanged<bool> onForceRetestChanged;

  const SettingsCard({
    super.key,
    this.timeoutSec = 5,
    this.maxConcurrent = 20,
    this.dns,
    this.debugMode = false,
    this.forceRetest = false,
    required this.onTimeoutChanged,
    required this.onConcurrentChanged,
    required this.onDnsChanged,
    required this.onDebugChanged,
    required this.onForceRetestChanged,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Settings', style: theme.textTheme.titleMedium),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: TextField(
                    decoration: const InputDecoration(
                      labelText: 'Timeout (s)',
                      isDense: true,
                    ),
                    controller: TextEditingController(text: timeoutSec.toString()),
                    keyboardType: TextInputType.number,
                    onChanged: (v) {
                      final n = int.tryParse(v);
                      if (n != null && n > 0) onTimeoutChanged(n);
                    },
                  ),
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: TextField(
                    decoration: const InputDecoration(
                      labelText: 'Max Concurrent',
                      isDense: true,
                    ),
                    controller: TextEditingController(text: maxConcurrent.toString()),
                    keyboardType: TextInputType.number,
                    onChanged: (v) {
                      final n = int.tryParse(v);
                      if (n != null && n > 0) onConcurrentChanged(n);
                    },
                  ),
                ),
              ],
            ),
            const SizedBox(height: 12),
            DropdownButtonFormField<String>(
              value: dns ?? 'auto',
              decoration: const InputDecoration(labelText: 'DNS', isDense: true),
              items: const [
                DropdownMenuItem(value: 'auto', child: Text('Auto')),
                DropdownMenuItem(value: 'doh', child: Text('DoH Only')),
                DropdownMenuItem(value: 'dot', child: Text('DoT Only')),
                DropdownMenuItem(value: 'udp', child: Text('UDP Only')),
              ],
              onChanged: (v) => onDnsChanged(v == 'auto' ? null : v),
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                Checkbox(value: debugMode, onChanged: onDebugChanged),
                const Text('Debug mode'),
                const SizedBox(width: 16),
                Checkbox(value: forceRetest, onChanged: onForceRetestChanged),
                const Text('Force retest'),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/features/home/views/widgets/settings_card.dart
git commit -m "feat(settings): add DNS dropdown, debug mode, force retest checkboxes"
```

---

### Task 10: Create AdvancedSettingsCard

**Files:**
- Create: `lib/ui/features/home/views/widgets/advanced_settings_card.dart`

- [ ] **Write AdvancedSettingsCard widget**

```dart
import 'package:flutter/material.dart';

class AdvancedSettingsCard extends StatefulWidget {
  final bool aimdEnabled;
  final int fixedWorkers;
  final int ttlDays;
  final int maxLines;
  final String? geoProxy;
  final bool autoShutdown;
  final ValueChanged<bool> onAimdChanged;
  final ValueChanged<int> onFixedWorkersChanged;
  final ValueChanged<int> onTtlChanged;
  final ValueChanged<int> onMaxLinesChanged;
  final ValueChanged<String> onProxyChanged;
  final ValueChanged<bool> onAutoShutdownChanged;

  const AdvancedSettingsCard({
    super.key,
    this.aimdEnabled = true,
    this.fixedWorkers = 20,
    this.ttlDays = 7,
    this.maxLines = 0,
    this.geoProxy,
    this.autoShutdown = false,
    required this.onAimdChanged,
    required this.onFixedWorkersChanged,
    required this.onTtlChanged,
    required this.onMaxLinesChanged,
    required this.onProxyChanged,
    required this.onAutoShutdownChanged,
  });

  @override
  State<AdvancedSettingsCard> createState() => _AdvancedSettingsCardState();
}

class _AdvancedSettingsCardState extends State<AdvancedSettingsCard> {
  bool _expanded = false;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            InkWell(
              onTap: () => setState(() => _expanded = !_expanded),
              child: Row(
                children: [
                  Text('Advanced Settings', style: theme.textTheme.titleMedium),
                  const Spacer(),
                  Icon(_expanded ? Icons.expand_less : Icons.expand_more),
                ],
              ),
            ),
            if (_expanded) ...[
              const SizedBox(height: 12),
              Row(
                children: [
                  const Text('Concurrency:'),
                  const SizedBox(width: 8),
                  DropdownButton<bool>(
                    value: widget.aimdEnabled,
                    items: const [
                      DropdownMenuItem(value: true, child: Text('AIMD Auto')),
                      DropdownMenuItem(value: false, child: Text('Fixed')),
                    ],
                    onChanged: (v) {
                      if (v != null) widget.onAimdChanged(v);
                    },
                  ),
                  if (!widget.aimdEnabled) ...[
                    const SizedBox(width: 8),
                    SizedBox(
                      width: 60,
                      child: TextField(
                        decoration: const InputDecoration(isDense: true),
                        controller: TextEditingController(
                            text: widget.fixedWorkers.toString()),
                        keyboardType: TextInputType.number,
                        onChanged: (v) {
                          final n = int.tryParse(v);
                          if (n != null && n > 0) widget.onFixedWorkersChanged(n);
                        },
                      ),
                    ),
                  ],
                ],
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  const Text('TTL (days):'),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 60,
                    child: TextField(
                      decoration: const InputDecoration(isDense: true),
                      controller:
                          TextEditingController(text: widget.ttlDays.toString()),
                      keyboardType: TextInputType.number,
                      onChanged: (v) {
                        final n = int.tryParse(v);
                        if (n != null && n >= 0) widget.onTtlChanged(n);
                      },
                    ),
                  ),
                  const SizedBox(width: 16),
                  const Text('Max lines:'),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 60,
                    child: TextField(
                      decoration: const InputDecoration(
                        isDense: true,
                        hintText: 'All',
                      ),
                      controller: TextEditingController(
                          text: widget.maxLines > 0 ? widget.maxLines.toString() : ''),
                      keyboardType: TextInputType.number,
                      onChanged: (v) {
                        final n = int.tryParse(v);
                        widget.onMaxLinesChanged(n ?? 0);
                      },
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 8),
              TextField(
                decoration: const InputDecoration(
                  labelText: 'GeoIP Proxy',
                  isDense: true,
                  hintText: 'socks5://127.0.0.1:1080',
                ),
                controller: TextEditingController(text: widget.geoProxy ?? ''),
                onChanged: widget.onProxyChanged,
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  Checkbox(
                    value: widget.autoShutdown,
                    onChanged: widget.onAutoShutdownChanged,
                  ),
                  const Text('Auto shutdown on completion'),
                ],
              ),
            ],
          ],
        ),
      ),
    );
  }
}
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/features/home/views/widgets/advanced_settings_card.dart
git commit -m "feat: add AdvancedSettingsCard with AIMD, TTL, proxy, auto-shutdown"
```

---

### Task 11: Create ResultDownloadCard

**Files:**
- Create: `lib/ui/features/home/views/widgets/result_download_card.dart`

- [ ] **Write ResultDownloadCard widget**

```dart
import 'dart:io';

import 'package:flutter/material.dart';

class ResultDownloadCard extends StatelessWidget {
  final bool loading;
  final double? progress;
  final String? savedPath;
  final VoidCallback onDownload;
  final VoidCallback onOpenFolder;

  const ResultDownloadCard({
    super.key,
    this.loading = false,
    this.progress,
    this.savedPath,
    required this.onDownload,
    required this.onOpenFolder,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    if (loading || savedPath != null) {
      return Card(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('Download Results', style: theme.textTheme.titleMedium),
              const SizedBox(height: 12),
              if (loading && progress != null) ...[
                LinearProgressIndicator(value: progress),
                const SizedBox(height: 8),
                Text('Downloading... ${(progress! * 100).toStringAsFixed(0)}%'),
              ] else if (savedPath != null) ...[
                Row(
                  children: [
                    const Icon(Icons.check_circle, color: Colors.green, size: 20),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text('Saved to $savedPath',
                          style: const TextStyle(fontSize: 12)),
                    ),
                    TextButton.icon(
                      onPressed: onOpenFolder,
                      icon: const Icon(Icons.folder_open, size: 18),
                      label: const Text('Open'),
                    ),
                  ],
                ),
              ],
            ],
          ),
        ),
      );
    }
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Row(
          children: [
            Text('Results', style: theme.textTheme.titleMedium),
            const Spacer(),
            FilledButton.tonalIcon(
              onPressed: onDownload,
              icon: const Icon(Icons.download, size: 18),
              label: const Text('Download'),
            ),
          ],
        ),
      ),
    );
  }
}
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/features/home/views/widgets/result_download_card.dart
git commit -m "feat: add ResultDownloadCard with progress and open folder"
```

---

### Task 12: Rewrite ViewModel with connection state machine

**Files:**
- Modify: `lib/ui/features/home/view_models/home_view_model.dart`

- [ ] **Rewrite HomeViewModel**

Replace entire content with:

```dart
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import '../../../../data/models/models.dart';
import '../../../../data/services/api_client.dart';
import '../../../../data/services/preferences_service.dart';
import '../../../core/platform.dart';

class HomeViewModel extends ChangeNotifier {
  final ApiClient api;
  final PreferencesService prefs;

  HomeViewModel({ApiClient? api, required PreferencesService prefs})
      : api = api ?? ApiClient(),
        prefs = prefs;

  // --- Connection state ---
  ConnMode _connMode = ConnMode.usb;
  ConnectionState _connState = ConnectionState.disconnected;
  String? _deviceName;
  String? _connError;
  String _remoteHost = 'localhost';
  int _remotePort = 18080;

  ConnMode get connMode => _connMode;
  ConnectionState get connState => _connState;
  String? get deviceName => _deviceName;
  String? get connError => _connError;
  String get remoteHost => _remoteHost;
  int get remotePort => _remotePort;
  bool get isConnected => _connState == ConnectionState.connected;

  // --- Phone mode ---
  PhoneMode _phoneMode = PhoneMode.local;
  PhoneMode get phoneMode => _phoneMode;

  // --- Remote files ---
  List<FileEntry> _remoteFiles = [];
  bool _filesLoading = false;
  double? _uploadProgress;

  List<FileEntry> get remoteFiles => _remoteFiles;
  bool get filesLoading => _filesLoading;
  double? get uploadProgress => _uploadProgress;

  // --- Test state (existing) ---
  Stats _stats = const Stats();
  String _status = 'idle';
  String? _message;
  bool _running = false;
  bool _initialized = false;
  List<ProgressEvent> _results = [];
  String? _error;
  StreamSubscription<ProgressEvent>? _sseSub;

  Stats get stats => _stats;
  String get status => _status;
  String? get message => _message;
  bool get running => _running;
  bool get initialized => _initialized;
  List<ProgressEvent> get results => _results;
  String? get error => _error;

  // --- Download state ---
  bool _downloadLoading = false;
  double? _downloadProgress;
  String? _downloadPath;

  bool get downloadLoading => _downloadLoading;
  double? get downloadProgress => _downloadProgress;
  String? get downloadPath => _downloadPath;

  // --- Settings ---
  int _timeoutSec = 5;
  int _maxConcurrent = 20;
  String? _dns;
  bool _debugMode = false;
  bool _forceRetest = false;
  bool _aimdEnabled = true;
  int _fixedWorkers = 20;
  int _ttlDays = 7;
  int _maxLines = 0;
  String? _geoProxy;
  bool _autoShutdown = false;

  int get timeoutSec => _timeoutSec;
  int get maxConcurrent => _maxConcurrent;
  String? get dns => _dns;
  bool get debugMode => _debugMode;
  bool get forceRetest => _forceRetest;
  bool get aimdEnabled => _aimdEnabled;
  int get fixedWorkers => _fixedWorkers;
  int get ttlDays => _ttlDays;
  int get maxLines => _maxLines;
  String? get geoProxy => _geoProxy;
  bool get autoShutdown => _autoShutdown;

  // --- Init ---
  Future<void> init() async {
    _loadSettings();
    _connMode = prefs.remoteConfig.mode;
    _remoteHost = prefs.remoteConfig.host;
    _remotePort = prefs.remoteConfig.port;
    if (_connMode == ConnMode.local) {
      try {
        await api.startBackend();
      } catch (e) {
        _error = 'Backend start failed: $e';
        notifyListeners();
        return;
      }
      _setConnected('Local');
    }
    _initialized = true;
    await refreshStatus();
  }

  void _loadSettings() {
    _timeoutSec = prefs.timeoutSec;
    _maxConcurrent = prefs.maxConcurrent;
    _dns = prefs.dns;
    _debugMode = prefs.debugMode;
    _forceRetest = prefs.forceRetest;
    _aimdEnabled = prefs.aimdEnabled;
    _fixedWorkers = prefs.fixedWorkers;
    _ttlDays = prefs.ttlDays;
    _maxLines = prefs.maxLines;
    _geoProxy = prefs.geoProxy;
    _autoShutdown = prefs.autoShutdown;
  }

  Future<void> updateSetting({int? timeoutSec, int? maxConcurrent, String? dns,
      bool? debugMode, bool? forceRetest, bool? aimdEnabled, int? fixedWorkers,
      int? ttlDays, int? maxLines, String? geoProxy, bool? autoShutdown}) async {
    if (timeoutSec != null) _timeoutSec = timeoutSec;
    if (maxConcurrent != null) _maxConcurrent = maxConcurrent;
    _dns = dns;
    if (debugMode != null) _debugMode = debugMode;
    if (forceRetest != null) _forceRetest = forceRetest;
    if (aimdEnabled != null) _aimdEnabled = aimdEnabled;
    if (fixedWorkers != null) _fixedWorkers = fixedWorkers;
    if (ttlDays != null) _ttlDays = ttlDays;
    if (maxLines != null) _maxLines = maxLines;
    if (geoProxy != null) _geoProxy = geoProxy;
    if (autoShutdown != null) _autoShutdown = autoShutdown;
    await prefs.saveAll(timeoutSec: _timeoutSec, maxConcurrent: _maxConcurrent,
        dns: _dns, debugMode: _debugMode, forceRetest: _forceRetest,
        aimdEnabled: _aimdEnabled, fixedWorkers: _fixedWorkers,
        ttlDays: _ttlDays, maxLines: _maxLines, geoProxy: _geoProxy,
        autoShutdown: _autoShutdown);
    notifyListeners();
  }

  // --- Connection ---
  void setConnMode(ConnMode mode) {
    _connMode = mode;
    if (mode == ConnMode.local) {
      _setDisconnected();
    }
    notifyListeners();
    prefs.saveRemoteConfig(RemoteConfig(mode: mode, host: _remoteHost, port: _remotePort));
  }

  void setRemoteHost(String h) {
    _remoteHost = h;
    prefs.saveRemoteConfig(RemoteConfig(mode: _connMode, host: h, port: _remotePort));
  }

  void setRemotePort(int p) {
    _remotePort = p;
    prefs.saveRemoteConfig(RemoteConfig(mode: _connMode, host: _remoteHost, port: p));
  }

  void setPhoneMode(PhoneMode mode) {
    _phoneMode = mode;
    notifyListeners();
  }

  Future<void> connect() async {
    _connState = ConnectionState.connecting;
    _connError = null;
    notifyListeners();

    try {
      if (_connMode == ConnMode.local) {
        await api.startBackend();
        _setConnected('Local');
      } else {
        final url = 'http://$_remoteHost:$_remotePort';
        await api.connectToRemote(url);
        _setConnected('$_remoteHost:$_remotePort');
        await refreshRemoteFiles();
      }
      await refreshStatus();
      prefs.saveRemoteConfig(
          RemoteConfig(mode: _connMode, host: _remoteHost, port: _remotePort));
    } catch (e) {
      _connState = ConnectionState.error;
      _connError = e.toString();
      notifyListeners();
    }
  }

  Future<void> disconnect() async {
    await api.stopBackend();
    _setDisconnected();
  }

  void _setConnected(String name) {
    _connState = ConnectionState.connected;
    _deviceName = name;
    _connError = null;
    notifyListeners();
  }

  void _setDisconnected() {
    _connState = ConnectionState.disconnected;
    _deviceName = null;
    _connError = null;
    notifyListeners();
  }

  // --- Remote files ---
  Future<void> refreshRemoteFiles() async {
    _filesLoading = true;
    notifyListeners();
    try {
      _remoteFiles = await api.listFiles();
    } catch (e) {
      _error = 'List files: $e';
    }
    _filesLoading = false;
    notifyListeners();
  }

  Future<void> uploadFile(String localPath) async {
    final name = localPath.split(Platform.pathSeparator).last;
    _uploadProgress = 0;
    notifyListeners();
    try {
      await api.uploadFile(localPath, name);
      _uploadProgress = 1.0;
      await refreshRemoteFiles();
    } catch (e) {
      _error = 'Upload failed: $e';
    }
    _uploadProgress = null;
    notifyListeners();
  }

  Future<void> deleteRemoteFile(String name) async {
    try {
      await api.deleteFile(name);
      await refreshRemoteFiles();
    } catch (e) {
      _error = 'Delete failed: $e';
      notifyListeners();
    }
  }

  // --- Test (existing) ---
  Future<void> refreshStatus() async {
    try {
      final s = await api.getStatus();
      _status = s.status;
      _stats = s.stats;
      _running = s.status == 'running';
      _message = s.message;
      _error = null;
    } catch (e) {
      _error = 'Status check: $e';
    }
    notifyListeners();
  }

  bool get canStart {
    if (_connMode != ConnMode.local && !isConnected) return false;
    return !_running;
  }

  Future<void> startTest(StartParams params) async {
    try {
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

  Future<void> stopTest() async {
    await _sseSub?.cancel();
    _sseSub = null;
    try {
      await api.stopTest();
    } catch (_) {}
    _running = false;
    await refreshStatus();
  }

  void _connectSSE() {
    _sseSub?.cancel();
    final stream = api.progressStream();
    _sseSub = stream.listen(
      (event) {
        _stats = event.stats;
        _results = [event, ..._results].take(200).toList();
        if (_stats.done >= _stats.total && _stats.total > 0) {
          _running = false;
          notifyListeners();
        }
        _error = null;
        notifyListeners();
      },
      onError: (e) {
        _error = 'SSE: $e';
        notifyListeners();
      },
    );
  }

  // --- Download ---
  Future<void> downloadResults() async {
    _downloadLoading = true;
    _downloadProgress = null;
    notifyListeners();
    try {
      final data = await api.downloadResult();
      final dir = Directory(
          '${(await getApplicationDocumentsDirectory()).path}/sni_tester_results');
      if (!dir.existsSync()) dir.createSync(recursive: true);
      final ts = DateTime.now().millisecondsSinceEpoch;
      final path = '${dir.path}/results_$ts.pb';
      await File(path).writeAsBytes(data);
      _downloadPath = path;
    } catch (e) {
      _error = 'Download failed: $e';
    }
    _downloadLoading = false;
    notifyListeners();
  }

  @override
  void dispose() {
    _sseSub?.cancel();
    api.dispose();
    super.dispose();
  }
}
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/features/home/view_models/home_view_model.dart
git commit -m "feat(vm): add connection state machine, remote files, download management"
```

---

### Task 13: Update HomeScreen with platform-conditional layout

**Files:**
- Modify: `lib/ui/features/home/views/home_screen.dart`

- [ ] **Rewrite HomeScreen**

Replace entire content with the full conditional layout:

(Note: the file is ~110 lines, here's the rewritten version — includes ConnectionCard for desktop, AppModeCard for phone, conditional file picker/download sections)

```dart
import 'package:flutter/material.dart';

import '../../../../data/models/models.dart';
import '../../../core/platform.dart';
import '../view_models/home_view_model.dart';
import 'widgets/advanced_settings_card.dart';
import 'widgets/app_mode_card.dart';
import 'widgets/connection_card.dart';
import 'widgets/file_picker_card.dart';
import 'widgets/progress_section.dart';
import 'widgets/remote_file_card.dart';
import 'widgets/result_download_card.dart';
import 'widgets/result_table.dart';
import 'widgets/settings_card.dart';

class HomeScreen extends StatefulWidget {
  final HomeViewModel viewModel;
  const HomeScreen({super.key, required this.viewModel});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  late final HomeViewModel _vm;
  String? _serversFile;
  String? _domainsFile;

  @override
  void initState() {
    super.initState();
    _vm = widget.viewModel;
    _vm.addListener(_onVmChanged);
    _vm.init();
  }

  void _onVmChanged() => setState(() {});

  @override
  void dispose() {
    _vm.removeListener(_onVmChanged);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('SNI Tester'),
        actions: [
          if (_vm.running)
            IconButton(
              icon: const Icon(Icons.stop),
              tooltip: 'Stop',
              onPressed: () => _vm.stopTest(),
            ),
        ],
      ),
      body: SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: Column(
          children: [
            // Connection / Mode card
            if (isDesktop)
              ConnectionCard(
                mode: _vm.connMode,
                connState: _vm.connState,
                deviceName: _vm.deviceName,
                errorMsg: _vm.connError,
                host: _vm.remoteHost,
                port: _vm.remotePort,
                onModeChanged: _vm.setConnMode,
                onConnect: _vm.connect,
                onDisconnect: _vm.disconnect,
                onHostChanged: _vm.setRemoteHost,
                onPortChanged: _vm.setRemotePort,
              ),
            if (isMobile)
              AppModeCard(
                mode: _vm.phoneMode,
                onModeChanged: _vm.setPhoneMode,
                backendRunning: _vm.initialized,
              ),
            const SizedBox(height: 12),

            // File selection
            if (isDesktop && _vm.connMode != ConnMode.local && _vm.isConnected)
              RemoteFileCard(
                files: _vm.remoteFiles,
                loading: _vm.filesLoading,
                onRefresh: _vm.refreshRemoteFiles,
                onUpload: _pickAndUploadFile,
                onDelete: _vm.deleteRemoteFile,
                uploadProgress: _vm.uploadProgress,
              ),
            if (isMobile || (_vm.connMode == ConnMode.local))
              FilePickerCard(
                serversFile: _serversFile,
                domainsFile: _domainsFile,
                onServersPicked: (p) => setState(() => _serversFile = p),
                onDomainsPicked: (p) => setState(() => _domainsFile = p),
              ),
            const SizedBox(height: 12),

            // Settings
            SettingsCard(
              timeoutSec: _vm.timeoutSec,
              maxConcurrent: _vm.maxConcurrent,
              dns: _vm.dns,
              debugMode: _vm.debugMode,
              forceRetest: _vm.forceRetest,
              onTimeoutChanged: (v) => _vm.updateSetting(timeoutSec: v),
              onConcurrentChanged: (v) => _vm.updateSetting(maxConcurrent: v),
              onDnsChanged: (v) => _vm.updateSetting(dns: v),
              onDebugChanged: (v) => _vm.updateSetting(debugMode: v),
              onForceRetestChanged: (v) => _vm.updateSetting(forceRetest: v),
            ),
            const SizedBox(height: 12),

            // Advanced (desktop only)
            if (isDesktop)
              AdvancedSettingsCard(
                aimdEnabled: _vm.aimdEnabled,
                fixedWorkers: _vm.fixedWorkers,
                ttlDays: _vm.ttlDays,
                maxLines: _vm.maxLines,
                geoProxy: _vm.geoProxy,
                autoShutdown: _vm.autoShutdown,
                onAimdChanged: (v) => _vm.updateSetting(aimdEnabled: v),
                onFixedWorkersChanged: (v) => _vm.updateSetting(fixedWorkers: v),
                onTtlChanged: (v) => _vm.updateSetting(ttlDays: v),
                onMaxLinesChanged: (v) => _vm.updateSetting(maxLines: v),
                onProxyChanged: (v) => _vm.updateSetting(geoProxy: v),
                onAutoShutdownChanged: (v) => _vm.updateSetting(autoShutdown: v),
              ),
            const SizedBox(height: 12),

            // Progress
            ProgressSection(
              running: _vm.running,
              stats: _vm.stats,
              error: _vm.error,
            ),
            const SizedBox(height: 12),

            // Start button
            SizedBox(
              width: double.infinity,
              height: 48,
              child: FilledButton.icon(
                onPressed: _vm.canStart
                    ? () => _vm.startTest(StartParams(
                          serversFile: _serversFile ?? '',
                          domainsFile: _domainsFile ?? '',
                          timeoutSec: _vm.timeoutSec,
                          maxConcurrent: _vm.maxConcurrent,
                        ))
                    : null,
                icon: Icon(_vm.running ? Icons.hourglass_top : Icons.play_arrow),
                label: Text(_vm.running ? 'Running...' : 'Start Test'),
              ),
            ),
            const SizedBox(height: 12),

            // Results table
            ResultTable(results: _vm.results),

            // Download (desktop only)
            if (isDesktop && !_vm.running && _vm.results.isNotEmpty)
              ResultDownloadCard(
                loading: _vm.downloadLoading,
                progress: _vm.downloadProgress,
                savedPath: _vm.downloadPath,
                onDownload: _vm.downloadResults,
                onOpenFolder: () {},
              ),
            const SizedBox(height: 80),
          ],
        ),
      ),
    );
  }

  Future<void> _pickAndUploadFile() async {
    final result = await FilePicker.platform.pickFiles();
    if (result != null && result.files.single.path != null) {
      await _vm.uploadFile(result.files.single.path!);
    }
  }
}
```

Note: This file depends on `AdvancedSettingsCard` and `ResultDownloadCard` created in Tasks 10-11. Also depends on `FilePicker` import. Ensure `file_picker` import is added at the top.

- [ ] **Fix import for FilePicker**

Add `import 'package:file_picker/file_picker.dart';` at top.

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/ui/features/home/views/home_screen.dart
git commit -m "feat(screen): platform-conditional layout with connection, remote files, advanced settings"
```

---

### Task 14: Update main.dart and pubspec.yaml

**Files:**
- Modify: `lib/main.dart`
- Modify: `pubspec.yaml`

- [ ] **Update pubspec.yaml**

Add `shared_preferences` under dependencies:

```yaml
  shared_preferences: ^2.2.0
```

- [ ] **Rewrite main.dart**

```dart
import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'data/services/preferences_service.dart';
import 'ui/core/theme.dart';
import 'ui/features/home/view_models/home_view_model.dart';
import 'ui/features/home/views/home_screen.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final sp = await SharedPreferences.getInstance();
  final prefService = PreferencesService(sp);
  runApp(SniTesterApp(prefService: prefService));
}

class SniTesterApp extends StatelessWidget {
  final PreferencesService prefService;
  const SniTesterApp({super.key, required this.prefService});

  @override
  Widget build(BuildContext context) {
    final vm = HomeViewModel(prefs: prefService, api: ApiClient());
    return MaterialApp(
      title: 'SNI Tester',
      debugShowCheckedModeBanner: false,
      theme: AppTheme.light,
      darkTheme: AppTheme.dark,
      themeMode: ThemeMode.system,
      home: HomeScreen(viewModel: vm),
    );
  }
}
```

- [ ] **Run pub get**

```bash
cd flutter_app && flutter pub get
```

- [ ] **Verify analyze passes**

Run: `cd flutter_app && flutter analyze`
Expected: 0 errors

- [ ] **Commit**

```bash
git add flutter_app/lib/main.dart flutter_app/pubspec.yaml flutter_app/pubspec.lock
git commit -m "feat: add shared_preferences, wire PreferencesService from main"
```

---

### Task 15: Update tests

**Files:**
- Modify: `test/models_test.dart`
- Create: `test/preferences_test.dart`

- [ ] **Add FileEntry model tests to test/models_test.dart**

Add after existing `StartParams` group:

```dart
  group('FileEntry', () {
    test('fromJson parses all fields', () {
      final j = {
        'name': 'domains.txt',
        'size': 12400,
        'mod_time': '2026-06-22T10:00:00Z',
      };
      final f = FileEntry.fromJson(j);
      expect(f.name, 'domains.txt');
      expect(f.size, 12400);
      expect(f.modTime, '2026-06-22T10:00:00Z');
    });

    test('sizeFormatted shows KB', () {
      final f = FileEntry(name: 'test.txt', size: 2048, modTime: '');
      expect(f.sizeFormatted, '2.0 KB');
    });

    test('sizeFormatted shows B', () {
      final f = FileEntry(name: 'test.txt', size: 500, modTime: '');
      expect(f.sizeFormatted, '500 B');
    });
  });

  group('StartParams with extras', () {
    test('toJson includes optional fields when set', () {
      final p = StartParams(
        serversFile: '/s.txt',
        domainsFile: '/d.txt',
        dns: 'doh',
        ttlDays: 14,
        forceRetest: true,
        debugMode: true,
        maxLines: 100,
        autoShutdown: true,
      );
      final j = p.toJson();
      expect(j['dns'], 'doh');
      expect(j['ttl_days'], 14);
      expect(j['force_retest'], true);
      expect(j['debug_mode'], true);
      expect(j['max_lines'], 100);
      expect(j['auto_shutdown'], true);
    });

    test('toJson excludes null optional fields', () {
      final p = StartParams(
        serversFile: '/s.txt',
        domainsFile: '/d.txt',
      );
      final j = p.toJson();
      expect(j.containsKey('dns'), false);
    });
  });
```

- [ ] **Create test/preferences_test.dart**

```dart
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:sni_tester/data/models/models.dart';
import 'package:sni_tester/data/services/preferences_service.dart';

void main() {
  group('PreferencesService', () {
    test('default remoteConfig', () async {
      SharedPreferences.setMockInitialValues({});
      final sp = await SharedPreferences.getInstance();
      final svc = PreferencesService(sp);
      final cfg = svc.remoteConfig;
      expect(cfg.mode, ConnMode.usb);
      expect(cfg.host, 'localhost');
      expect(cfg.port, 18080);
    });

    test('save and load remoteConfig', () async {
      SharedPreferences.setMockInitialValues({});
      final sp = await SharedPreferences.getInstance();
      final svc = PreferencesService(sp);
      await svc.saveRemoteConfig(
          RemoteConfig(mode: ConnMode.wifi, host: '192.168.1.5', port: 9090));
      final cfg = svc.remoteConfig;
      expect(cfg.mode, ConnMode.wifi);
      expect(cfg.host, '192.168.1.5');
      expect(cfg.port, 9090);
    });
  });
}
```

- [ ] **Run tests**

Run: `cd flutter_app && flutter test`
Expected: All tests pass

- [ ] **Commit**

```bash
git add flutter_app/test/
git commit -m "test: add FileEntry model tests, PreferencesService tests, StartParams extras"
```

---

### Task 16: Update Makefile

**Files:**
- Modify: `Makefile`

- [ ] **Add flutter-linux-run target that builds Go assets first**

```makefile
flutter-linux-run:
	go build -o flutter_app/assets/sni_web ./cmd/sni_web/
	cd flutter_app && flutter run -d linux
```

Add this after the existing `linux-run` target.

- [ ] **Commit**

```bash
git add Makefile
git commit -m "chore: add flutter-linux-run target"
```

---

## Self-Review Checklist

- [ ] Every spec requirement maps to at least one task
- [ ] No TBD/TODO/incomplete code blocks
- [ ] Type/method names consistent across files (ConnMode, ConnectionState, FileEntry, PreferencesService)
- [ ] All imports needed by the code are listed or implied
- [ ] Each task produces a compilable/intermediate state
- [ ] Test files exist for models, preferences, and extended params
