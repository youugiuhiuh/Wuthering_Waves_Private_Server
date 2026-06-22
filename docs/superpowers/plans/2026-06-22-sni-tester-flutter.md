# SNI Tester Flutter Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Flutter mobile app as the frontend for sni_tester's Go backend, giving a beautiful cross-platform UI with real-time progress, file upload, and result download.

**Architecture:** MVVM (Model-View-ViewModel) per Flutter architecture best practices. Go backend (pkg/ + cmd/sni_web) serves HTTP/SSE API on localhost:18080. Flutter app starts the Go binary as a subprocess on app launch, communicates via REST + SSE. Single-screen design with file picker, settings, progress bar, and result table.

**Tech Stack:** Go (net/http, SSE), Flutter (Material 3, ChangeNotifier, go_router), dart:io subprocess management.

---

## File Structure

```
sni_tester/
├── cmd/sni_web/
│   ├── main.go              # MODIFY: remove embed, add CORS, add /api/health, port 18080
│   ├── handlers.go          # UNCHANGED
│   └── static/              # DELETE (moved to Flutter)
├── flutter_app/
│   ├── assets/
│   │   └── .gitkeep         # Go binary copied here at build time
│   ├── lib/
│   │   ├── main.dart        # Entry, Go process manager, MaterialApp.router
│   │   ├── data/
│   │   │   ├── models/
│   │   │   │   └── models.dart        # ProgressEvent, Stats, StatusResponse, StartParams
│   │   │   └── services/
│   │   │       └── api_client.dart     # HTTP client + SSE stream + Go process manager
│   │   └── ui/
│   │       ├── core/
│   │       │   └── theme.dart          # Material 3 theme, system dark/light
│   │       └── features/
│   │           └── home/
│   │               ├── view_models/
│   │               │   └── home_view_model.dart  # ChangeNotifier: all UI state
│   │               └── views/
│   │                   ├── home_screen.dart      # Main screen, assembles all widgets
│   │                   ├── file_picker_card.dart  # File upload widget
│   │                   ├── settings_card.dart     # Collapsible settings
│   │                   ├── progress_section.dart  # Progress bar + stats
│   │                   └── result_table.dart      # Live result list
│   ├── test/
│   │   ├── api_client_test.dart
│   │   └── widget_test.dart
│   └── pubspec.yaml
└── Makefile                 # MODIFY: add flutter-build / flutter-deploy targets
```

---

### Task 1: Go backend — pure API mode

**Files:**
- Modify: `sni_tester/cmd/sni_web/main.go`
- Delete: `sni_tester/cmd/sni_web/static/index.html`

- [ ] **Step 1: Rewrite cmd/sni_web/main.go**

Remove embed and static file serving. Add CORS middleware. Add `/api/health` endpoint. Change port to 18080.

```go
package main

import (
	"encoding/json"
	"log"
	"net/http"
	"os"
	"time"

	"sni_tester/pkg"
)

var startTime = time.Now()

func cors(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Access-Control-Allow-Origin", "*")
		w.Header().Set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "Content-Type")
		if r.Method == "OPTIONS" {
			w.WriteHeader(204)
			return
		}
		next.ServeHTTP(w, r)
	})
}

func main() {
	outputDir := "/data/local/tmp/sni_output"
	if d := os.Getenv("SNI_OUTPUT_DIR"); d != "" {
		outputDir = d
	}

	cfg := pkg.DefaultConfig()
	cfg.OutputDir = outputDir
	cfg.Debug = true

	if err := pkg.PrepareGeoDBs(cfg.GeoDBFile, cfg.GeoASNFile, ""); err != nil {
		log.Printf("Warning: GeoDB download failed: %v", err)
	}

	engine, err := pkg.NewEngine(cfg)
	if err != nil {
		log.Fatalf("Failed to init engine: %v", err)
	}
	defer engine.Close()

	srv := &Server{
		engine:      engine,
		cfg:         cfg,
		subscribers: make(map[chan pkg.ProgressEvent]struct{}),
	}

	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/health", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]interface{}{
			"status":  "ok",
			"uptime":  time.Since(startTime).String(),
			"version": "1.0",
		})
	})
	mux.HandleFunc("GET /api/progress", srv.handleSSE)
	mux.HandleFunc("POST /api/start", srv.handleStart)
	mux.HandleFunc("POST /api/stop", srv.handleStop)
	mux.HandleFunc("GET /api/status", srv.handleStatus)
	mux.HandleFunc("GET /api/download", srv.handleDownload)
	mux.HandleFunc("POST /api/upload", srv.handleUpload)

	log.Println("SNI API: http://0.0.0.0:18080")
	log.Fatal(http.ListenAndServe("0.0.0.0:18080", cors(mux)))
}
```

- [ ] **Step 2: Delete static/ directory**

```bash
rm -rf sni_tester/cmd/sni_web/static/
```

- [ ] **Step 3: Build and verify**

```bash
cd sni_tester && go build ./cmd/sni_web/
```

Expected: `sni_web` binary compiles cleanly, no embed errors.

- [ ] **Step 4: Run existing tests**

```bash
cd sni_tester && go test ./pkg/ -v
```

Expected: 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add sni_tester/cmd/sni_web/
git rm -r sni_tester/cmd/sni_web/static/
git commit -m "refactor(sni_web): pure API mode with CORS + health endpoint, port 18080"
```

---

### Task 2: Flutter project initialization + models

**Files:**
- Create: `sni_tester/flutter_app/pubspec.yaml`
- Create: `sni_tester/flutter_app/lib/data/models/models.dart`
- Create: `sni_tester/flutter_app/analysis_options.yaml`

- [ ] **Step 1: Create pubspec.yaml**

```yaml
name: sni_tester_app
description: SNI Tester Flutter frontend
publish_to: none
version: 1.0.0+1

environment:
  sdk: ^3.5.0

dependencies:
  flutter:
    sdk: flutter
  http: ^1.2.0
  file_picker: ^8.0.0
  path_provider: ^2.1.0
  share_plus: ^9.0.0
  go_router: ^14.0.0

dev_dependencies:
  flutter_test:
    sdk: flutter
  flutter_lints: ^4.0.0

flutter:
  uses-material-design: true
  assets:
    - assets/
```

- [ ] **Step 2: Create analysis_options.yaml**

```yaml
include: package:flutter_lints/flutter.yaml

linter:
  rules:
    prefer_const_constructors: true
    prefer_const_declarations: true
    avoid_print: false
```

- [ ] **Step 3: Write models.dart**

```dart
class Stats {
  final int total;
  final int success;
  final int failed;
  final int skipped;
  final double ratePerSec;

  Stats({this.total = 0, this.success = 0, this.failed = 0,
         this.skipped = 0, this.ratePerSec = 0.0});

  factory Stats.fromJson(Map<String, dynamic> json) => Stats(
    total: json['total'] ?? 0,
    success: json['success'] ?? 0,
    failed: json['failed'] ?? 0,
    skipped: json['skipped'] ?? 0,
    ratePerSec: (json['ratePerSec'] ?? 0.0).toDouble(),
  );

  int get done => success + failed + skipped;
  double get progress => total > 0 ? done / total : 0.0;
}

class ProgressEvent {
  final String type;
  final String domain;
  final bool success;
  final String country;
  final String ip;
  final String info;
  final double progress;
  final Stats stats;

  ProgressEvent({this.type = '', this.domain = '', this.success = false,
    this.country = '', this.ip = '', this.info = '',
    this.progress = 0.0, Stats? stats})
    : stats = stats ?? Stats();

  factory ProgressEvent.fromJson(Map<String, dynamic> json) => ProgressEvent(
    type: json['type'] ?? '',
    domain: json['domain'] ?? '',
    success: json['success'] ?? false,
    country: json['country'] ?? '',
    ip: json['ip'] ?? '',
    info: json['info'] ?? '',
    progress: (json['progress'] ?? 0.0).toDouble(),
    stats: json['stats'] != null
        ? Stats.fromJson(json['stats'])
        : Stats(),
  );
}

class StatusResponse {
  final bool running;
  final Stats? stats;

  StatusResponse({this.running = false, this.stats});

  factory StatusResponse.fromJson(Map<String, dynamic> json) => StatusResponse(
    running: json['running'] ?? false,
    stats: json['stats'] != null ? Stats.fromJson(json['stats']) : null,
  );
}

class StartParams {
  int? workers;
  String? dns;
  int? ttl;
  bool force;
  bool reset;

  StartParams({this.workers, this.dns, this.ttl,
    this.force = false, this.reset = false});

  Map<String, dynamic> toJson() => {
    if (workers != null) 'workers': workers,
    if (dns != null) 'dns': dns,
    if (ttl != null) 'ttl': ttl,
    'force': force,
    'reset': reset,
  };
}
```

- [ ] **Step 4: Create assets/.gitkeep**

```bash
mkdir -p sni_tester/flutter_app/assets
touch sni_tester/flutter_app/assets/.gitkeep
```

- [ ] **Step 5: Commit**

```bash
git add sni_tester/flutter_app/
git commit -m "feat(flutter): init Flutter project with data models"
```

---

### Task 3: API Client + Go process manager

**Files:**
- Create: `sni_tester/flutter_app/lib/data/services/api_client.dart`

- [ ] **Step 1: Write api_client.dart**

```dart
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:http/http.dart' as http;
import 'package:path_provider/path_provider.dart';

import '../models/models.dart';

class ApiClient {
  static const String _baseUrl = 'http://localhost:18080';
  final http.Client _http = http.Client();

  // ── Go process management ──

  Future<void> ensureBackend() async {
    if (await healthCheck()) return;
    final dir = await getApplicationDocumentsDirectory();
    final binary = File('${dir.path}/sni_web');
    if (!await binary.exists()) {
      // Copy from app assets
      final asset = File('assets/sni_web');
      if (await asset.exists()) {
        await asset.copy(binary.path);
      }
    }
    if (await binary.exists()) {
      await binary.setExecutableElement(true);
      Process.start(binary.path, [], environment: {
        'SNI_OUTPUT_DIR': '${dir.path}/sni_output',
      });
      // Wait for backend to start (up to 5s)
      for (var i = 0; i < 50; i++) {
        await Future.delayed(const Duration(milliseconds: 100));
        if (await healthCheck()) return;
      }
    }
  }

  Future<bool> healthCheck() async {
    try {
      final r = await _http.get(Uri.parse('$_baseUrl/api/health'))
          .timeout(const Duration(seconds: 2));
      return r.statusCode == 200;
    } catch (_) {
      return false;
    }
  }

  // ── API methods ──

  Future<StatusResponse> getStatus() async {
    final r = await _http.get(Uri.parse('$_baseUrl/api/status'));
    return StatusResponse.fromJson(jsonDecode(r.body));
  }

  Future<void> upload(File file) async {
    final req = http.MultipartRequest('POST', Uri.parse('$_baseUrl/api/upload'));
    req.files.add(await http.MultipartFile.fromPath('file', file.path));
    final r = await req.send();
    if (r.statusCode != 200) throw Exception('Upload failed: ${r.statusCode}');
  }

  Future<void> start(StartParams params) async {
    final r = await _http.post(
      Uri.parse('$_baseUrl/api/start'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode(params.toJson()),
    );
    if (r.statusCode != 200) throw Exception('Start failed: ${r.statusCode}');
  }

  Future<void> stop() async {
    await _http.post(Uri.parse('$_baseUrl/api/stop'));
  }

  Stream<ProgressEvent> progressStream() {
    final ctrl = StreamController<ProgressEvent>();
    http.Client().send(http.Request('GET',
        Uri.parse('$_baseUrl/api/progress'))).then((r) async {
      r.stream.transform(utf8.decoder).transform(const LineSplitter())
          .listen((line) {
        if (line.startsWith('data: ')) {
          try {
            final json = jsonDecode(line.substring(6)) as Map<String, dynamic>;
            ctrl.add(ProgressEvent.fromJson(json));
          } catch (_) {}
        }
      }, onDone: () => ctrl.close(), onError: (e) => ctrl.addError(e));
    });
    return ctrl.stream;
  }

  Future<File> download(String savePath) async {
    final r = await _http.get(Uri.parse('$_baseUrl/api/download'));
    final file = File(savePath);
    await file.writeAsBytes(r.bodyBytes);
    return file;
  }

  void dispose() => _http.close();
}
```

- [ ] **Step 2: Commit**

```bash
git add sni_tester/flutter_app/lib/data/services/api_client.dart
git commit -m "feat(flutter): add API client with Go process manager"
```

---

### Task 4: UI theme + ViewModel

**Files:**
- Create: `sni_tester/flutter_app/lib/ui/core/theme.dart`
- Create: `sni_tester/flutter_app/lib/ui/features/home/view_models/home_view_model.dart`

- [ ] **Step 1: Write theme.dart**

```dart
import 'package:flutter/material.dart';

class AppTheme {
  static ThemeData get light => ThemeData(
    useMaterial3: true,
    colorSchemeSeed: Colors.indigo,
    brightness: Brightness.light,
  );

  static ThemeData get dark => ThemeData(
    useMaterial3: true,
    colorSchemeSeed: Colors.indigo,
    brightness: Brightness.dark,
  );
}
```

- [ ] **Step 2: Write home_view_model.dart**

```dart
import 'dart:io';
import 'dart:async';
import 'package:flutter/foundation.dart';
import '../../../data/models/models.dart';
import '../../../data/services/api_client.dart';

enum AppState { initializing, idle, running, done, error }

class HomeViewModel extends ChangeNotifier {
  final ApiClient _api = ApiClient();

  AppState state = AppState.initializing;
  String? fileName;
  int domainCount = 0;
  Stats stats = Stats();
  double progress = 0.0;
  final List<ProgressEvent> results = [];
  String? errorMessage;

  // Settings
  int workers = 200;
  String dns = '';
  int ttl = 7;
  bool force = false;
  bool reset = false;
  bool settingsExpanded = false;
  File? _uploadedFile;

  StreamSubscription<ProgressEvent>? _progressSub;

  Future<void> init() async {
    try {
      state = AppState.initializing;
      notifyListeners();
      await _api.ensureBackend();
      final status = await _api.getStatus();
      state = status.running ? AppState.running : AppState.idle;
    } catch (e) {
      state = AppState.error;
      errorMessage = 'Failed to start backend: $e';
    }
    notifyListeners();
  }

  void selectFile(File file, int count) {
    _uploadedFile = file;
    fileName = file.path.split('/').last;
    domainCount = count;
    notifyListeners();
  }

  Future<void> start() async {
    if (_uploadedFile == null) return;
    try {
      await _api.upload(_uploadedFile!);
      results.clear();
      stats = Stats();
      progress = 0.0;
      state = AppState.running;
      notifyListeners();

      await _api.start(StartParams(
        workers: workers,
        dns: dns.isEmpty ? null : dns,
        ttl: ttl,
        force: force,
        reset: reset,
      ));

      _progressSub = _api.progressStream().listen((event) {
        stats = event.stats;
        progress = event.progress;
        if (event.domain.isNotEmpty && event.type == 'result') {
          results.insert(0, event);
          if (results.length > 100) results.removeLast();
        }
        notifyListeners();
        if (event.type == 'done') {
          state = AppState.done;
          notifyListeners();
        }
      });
    } catch (e) {
      state = AppState.error;
      errorMessage = '$e';
      notifyListeners();
    }
  }

  Future<void> stop() async {
    await _api.stop();
    await _progressSub?.cancel();
    state = AppState.idle;
    notifyListeners();
  }

  Future<File?> download() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final path = '${dir.path}/sni_results_${DateTime.now().millisecondsSinceEpoch}.zip';
      return await _api.download(path);
    } catch (e) {
      errorMessage = 'Download failed: $e';
      notifyListeners();
      return null;
    }
  }

  void toggleSettings() {
    settingsExpanded = !settingsExpanded;
    notifyListeners();
  }

  @override
  void dispose() {
    _progressSub?.cancel();
    _api.dispose();
    super.dispose();
  }
}
```

- [ ] **Step 3: Commit**

```bash
git add sni_tester/flutter_app/lib/ui/
git commit -m "feat(flutter): add theme and HomeViewModel with full state management"
```

---

### Task 5: UI Widgets

**Files:**
- Create: `sni_tester/flutter_app/lib/ui/features/home/views/file_picker_card.dart`
- Create: `sni_tester/flutter_app/lib/ui/features/home/views/settings_card.dart`
- Create: `sni_tester/flutter_app/lib/ui/features/home/views/progress_section.dart`
- Create: `sni_tester/flutter_app/lib/ui/features/home/views/result_table.dart`

- [ ] **Step 1: Write file_picker_card.dart**

```dart
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:file_picker/file_picker.dart';
import '../view_models/home_view_model.dart';

class FilePickerCard extends StatelessWidget {
  final HomeViewModel vm;
  const FilePickerCard(this.vm, {super.key});

  @override
  Widget build(BuildContext context) {
    return Card(
      child: InkWell(
        onTap: _pickFile,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: vm.fileName == null
              ? Column(
                  children: [
                    Icon(Icons.upload_file, size: 40,
                        color: Theme.of(context).colorScheme.primary),
                    const SizedBox(height: 8),
                    Text('Tap to select domains file',
                        style: Theme.of(context).textTheme.bodyLarge),
                    Text('TXT / CSV',
                        style: Theme.of(context).textTheme.bodySmall),
                  ],
                )
              : Row(
                  children: [
                    Icon(Icons.description, color: Theme.of(context).colorScheme.primary),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(vm.fileName!, style: const TextStyle(fontWeight: FontWeight.w600)),
                          Text('${vm.domainCount} domains',
                              style: Theme.of(context).textTheme.bodySmall),
                        ],
                      ),
                    ),
                    IconButton(
                      icon: const Icon(Icons.close),
                      onPressed: () => vm.selectFile(null as File, 0),
                    ),
                  ],
                ),
        ),
      ),
    );
  }

  Future<void> _pickFile() async {
    final result = await FilePicker.platform.pickFiles(
      type: FileType.custom,
      allowedExtensions: ['txt', 'csv'],
    );
    if (result != null && result.files.single.path != null) {
      final file = File(result.files.single.path!);
      final count = await file.readAsLines().then((l) => l.length);
      vm.selectFile(file, count);
    }
  }
}
```

- [ ] **Step 2: Write settings_card.dart**

```dart
import 'package:flutter/material.dart';
import '../view_models/home_view_model.dart';

class SettingsCard extends StatelessWidget {
  final HomeViewModel vm;
  const SettingsCard(this.vm, {super.key});

  @override
  Widget build(BuildContext context) {
    return Card(
      child: Column(
        children: [
          ListTile(
            leading: const Icon(Icons.tune),
            title: const Text('Settings'),
            trailing: Icon(vm.settingsExpanded ? Icons.expand_less : Icons.expand_more),
            onTap: vm.toggleSettings,
          ),
          if (vm.settingsExpanded)
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
              child: Column(
                children: [
                  _slider(context, 'Workers', vm.workers, 10, 2000,
                      (v) => vm.workers = v),
                  TextField(
                    decoration: const InputDecoration(
                      labelText: 'DNS Server (optional)',
                      border: OutlineInputBorder(),
                    ),
                    onChanged: (v) => vm.dns = v,
                  ),
                  const SizedBox(height: 8),
                  _slider(context, 'TTL (days)', vm.ttl, 1, 30,
                      (v) => vm.ttl = v),
                  Row(
                    children: [
                      Switch(value: vm.force, onChanged: (v) { vm.force = v; vm.notifyListeners(); }),
                      const Text('Force retry'),
                      const SizedBox(width: 16),
                      Switch(value: vm.reset, onChanged: (v) { vm.reset = v; vm.notifyListeners(); }),
                      const Text('Reset history'),
                    ],
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }

  Widget _slider(BuildContext context, String label, int value,
      int min, int max, ValueChanged<int> onChanged) {
    return Row(
      children: [
        SizedBox(width: 100, child: Text(label)),
        Expanded(
          child: Slider(
            value: value.toDouble(),
            min: min.toDouble(),
            max: max.toDouble(),
            divisions: max - min,
            onChanged: (v) => onChanged(v.round()),
          ),
        ),
        SizedBox(width: 40, child: Text('$value')),
      ],
    );
  }
}
```

- [ ] **Step 3: Write progress_section.dart**

```dart
import 'package:flutter/material.dart';
import '../view_models/home_view_model.dart';

class ProgressSection extends StatelessWidget {
  final HomeViewModel vm;
  const ProgressSection(this.vm, {super.key});

  @override
  Widget build(BuildContext context) {
    final total = vm.stats.total;
    final done = vm.stats.done;
    final progress = total > 0 ? done / total : 0.0;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (total > 0) ...[
              ClipRRect(
                borderRadius: BorderRadius.circular(4),
                child: LinearProgressIndicator(
                  value: progress,
                  minHeight: 8,
                  backgroundColor: Theme.of(context).colorScheme.surfaceContainerHighest,
                ),
              ),
              const SizedBox(height: 12),
            ],
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceAround,
              children: [
                _stat('Total', '${vm.stats.total}', Colors.grey),
                _stat('Success', '${vm.stats.success}', Colors.green),
                _stat('Failed', '${vm.stats.failed}', Colors.red),
                _stat('Skipped', '${vm.stats.skipped}', Colors.orange),
              ],
            ),
            if (vm.stats.ratePerSec > 0)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Center(
                  child: Text('${vm.stats.ratePerSec.toStringAsFixed(1)} domains/s',
                      style: Theme.of(context).textTheme.bodySmall),
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _stat(String label, String value, Color color) {
    return Column(
      children: [
        Text(value, style: TextStyle(
            fontSize: 20, fontWeight: FontWeight.bold, color: color)),
        Text(label, style: const TextStyle(fontSize: 12)),
      ],
    );
  }
}
```

- [ ] **Step 4: Write result_table.dart**

```dart
import 'package:flutter/material.dart';
import '../view_models/home_view_model.dart';

class ResultTable extends StatelessWidget {
  final HomeViewModel vm;
  const ResultTable(this.vm, {super.key});

  @override
  Widget build(BuildContext context) {
    if (vm.results.isEmpty) return const SizedBox.shrink();
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.only(left: 8, top: 4, bottom: 4),
              child: Text('Results (${vm.results.length})',
                  style: Theme.of(context).textTheme.titleSmall),
            ),
            SizedBox(
              height: 300,
              child: ListView.builder(
                itemCount: vm.results.length,
                itemBuilder: (context, i) {
                  final r = vm.results[i];
                  return ListTile(
                    dense: true,
                    leading: Icon(
                      r.success ? Icons.check_circle : Icons.cancel,
                      color: r.success ? Colors.green : Colors.red,
                      size: 20,
                    ),
                    title: Text(r.domain, style: const TextStyle(fontSize: 13)),
                    subtitle: r.country.isNotEmpty
                        ? Text('${r.country}  ${r.ip}', style: const TextStyle(fontSize: 11))
                        : null,
                    trailing: r.info.isNotEmpty
                        ? Text(r.info, style: const TextStyle(fontSize: 11, color: Colors.grey))
                        : null,
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}
```

- [ ] **Step 5: Commit**

```bash
git add sni_tester/flutter_app/lib/ui/features/home/views/
git commit -m "feat(flutter): add UI widgets - file picker, settings, progress, result table"
```

---

### Task 6: HomeScreen + main.dart entry point

**Files:**
- Create: `sni_tester/flutter_app/lib/ui/features/home/views/home_screen.dart`
- Create: `sni_tester/flutter_app/lib/main.dart`

- [ ] **Step 1: Write home_screen.dart**

```dart
import 'package:flutter/material.dart';
import 'package:share_plus/share_plus.dart';
import '../view_models/home_view_model.dart';
import 'file_picker_card.dart';
import 'settings_card.dart';
import 'progress_section.dart';
import 'result_table.dart';

class HomeScreen extends StatefulWidget {
  final HomeViewModel vm;
  const HomeScreen(this.vm, {super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  @override
  void initState() {
    super.initState();
    widget.vm.addListener(_onChanged);
    widget.vm.init();
  }

  @override
  void dispose() {
    widget.vm.removeListener(_onChanged);
    super.dispose();
  }

  void _onChanged() => setState(() {});

  @override
  Widget build(BuildContext context) {
    final vm = widget.vm;
    return Scaffold(
      appBar: AppBar(
        title: const Text('SNI Tester'),
        actions: [
          _statusIndicator(vm.state),
          const SizedBox(width: 12),
        ],
      ),
      body: _buildBody(vm),
      floatingActionButton: _buildFab(vm),
    );
  }

  Widget _buildBody(HomeViewModel vm) {
    if (vm.state == AppState.initializing) {
      return const Center(child: CircularProgressIndicator());
    }
    if (vm.state == AppState.error) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(Icons.error_outline, size: 48, color: Colors.red),
            const SizedBox(height: 16),
            Text(vm.errorMessage ?? 'Unknown error'),
            const SizedBox(height: 16),
            ElevatedButton(onPressed: vm.init, child: const Text('Retry')),
          ],
        ),
      );
    }
    return ListView(
      padding: const EdgeInsets.all(12),
      children: [
        FilePickerCard(vm),
        const SizedBox(height: 8),
        SettingsCard(vm),
        const SizedBox(height: 8),
        ProgressSection(vm),
        const SizedBox(height: 8),
        ResultTable(vm),
        const SizedBox(height: 80),
      ],
    );
  }

  Widget? _buildFab(HomeViewModel vm) {
    if (vm.state == AppState.initializing) return null;
    if (vm.state == AppState.running) {
      return FloatingActionButton.extended(
        onPressed: vm.stop,
        icon: const Icon(Icons.stop),
        label: const Text('Stop'),
        backgroundColor: Colors.red,
      );
    }
    if (vm.state == AppState.done) {
      return Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          FloatingActionButton.extended(
            onPressed: () async {
              final file = await vm.download();
              if (file != null && mounted) {
                await Share.shareXFiles([XFile(file.path)],
                    text: 'SNI Tester Results');
              }
            },
            icon: const Icon(Icons.download),
            label: const Text('Download & Share'),
            heroTag: 'download',
          ),
          const SizedBox(height: 12),
          FloatingActionButton.extended(
            onPressed: () async {
              await vm.stop();
              vm.results.clear();
              vm.progress = 0.0;
              vm.stats = Stats();
              setState(() {});
            },
            icon: const Icon(Icons.refresh),
            label: const Text('New Test'),
            heroTag: 'new',
          ),
        ],
      );
    }
    return FloatingActionButton.extended(
      onPressed: vm.fileName == null ? null : vm.start,
      icon: const Icon(Icons.play_arrow),
      label: const Text('Start'),
    );
  }

  Widget _statusIndicator(AppState state) {
    Color color;
    switch (state) {
      case AppState.running: color = Colors.orange; break;
      case AppState.done: color = Colors.green; break;
      case AppState.error: color = Colors.red; break;
      default: color = Colors.grey;
    }
    return Container(
      width: 12, height: 12,
      decoration: BoxDecoration(shape: BoxShape.circle, color: color),
    );
  }
}
```

- [ ] **Step 2: Write main.dart**

```dart
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'ui/core/theme.dart';
import 'ui/features/home/view_models/home_view_model.dart';
import 'ui/features/home/views/home_screen.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  SystemChrome.setPreferredOrientations([
    DeviceOrientation.portraitUp,
    DeviceOrientation.portraitDown,
  ]);
  runApp(SNITesterApp());
}

class SNITesterApp extends StatefulWidget {
  @override
  State<SNITesterApp> createState() => _SNITesterAppState();
}

class _SNITesterAppState extends State<SNITesterApp> {
  final HomeViewModel _vm = HomeViewModel();

  @override
  void dispose() {
    _vm.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'SNI Tester',
      theme: AppTheme.light,
      darkTheme: AppTheme.dark,
      themeMode: ThemeMode.system,
      home: HomeScreen(_vm),
      debugShowCheckedModeBanner: false,
    );
  }
}
```

- [ ] **Step 3: Verify no syntax errors** (run `dart analyze` if available)

```bash
cd sni_tester/flutter_app && dart analyze lib/
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add sni_tester/flutter_app/lib/main.dart
git add sni_tester/flutter_app/lib/ui/features/home/views/home_screen.dart
git commit -m "feat(flutter): add HomeScreen and main.dart entry point"
```

---

### Task 7: Widget tests

**Files:**
- Create: `sni_tester/flutter_app/test/api_client_test.dart`
- Create: `sni_tester/flutter_app/test/widget_test.dart`

- [ ] **Step 1: Write api_client_test.dart**

```dart
import 'package:flutter_test/flutter_test.dart';
import 'package:sni_tester_app/data/models/models.dart';

void main() {
  group('Stats', () {
    test('fromJson parses correctly', () {
      final s = Stats.fromJson({'total': 100, 'success': 60, 'failed': 10, 'skipped': 5, 'ratePerSec': 5.5});
      expect(s.total, 100);
      expect(s.success, 60);
      expect(s.failed, 10);
      expect(s.done, 75);
      expect(s.progress, 0.75);
    });

    test('default values', () {
      final s = Stats();
      expect(s.total, 0);
      expect(s.progress, 0.0);
    });
  });

  group('ProgressEvent', () {
    test('fromJson with full data', () {
      final json = {
        'type': 'result',
        'domain': 'example.com',
        'success': true,
        'country': 'US',
        'ip': '1.2.3.4',
        'info': 'OK',
        'progress': 0.5,
        'stats': {'total': 10, 'success': 5, 'failed': 0, 'skipped': 0, 'ratePerSec': 2.0},
      };
      final e = ProgressEvent.fromJson(json);
      expect(e.domain, 'example.com');
      expect(e.success, true);
      expect(e.country, 'US');
      expect(e.stats.total, 10);
    });
  });

  group('StartParams', () {
    test('toJson includes non-null fields', () {
      final p = StartParams(workers: 200, force: true);
      final json = p.toJson();
      expect(json['workers'], 200);
      expect(json['force'], true);
      expect(json.containsKey('dns'), false);
    });
  });
}
```

- [ ] **Step 2: Write widget_test.dart**

```dart
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sni_tester_app/ui/core/theme.dart';

void main() {
  testWidgets('App renders theme correctly', (WidgetTester tester) async {
    await tester.pumpWidget(MaterialApp(
      theme: AppTheme.light,
      home: const Scaffold(body: Center(child: Text('SNI Tester'))),
    ));
    expect(find.text('SNI Tester'), findsOneWidget);
  });
}
```

- [ ] **Step 3: Run tests**

```bash
cd sni_tester/flutter_app && flutter test
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add sni_tester/flutter_app/test/
git commit -m "test(flutter): add model unit tests and widget smoke test"
```

---

### Task 8: Makefile update — flutter-build and flutter-deploy targets

**Files:**
- Modify: `sni_tester/Makefile`

- [ ] **Step 1: Update Makefile**

Add flutter-build and flutter-deploy targets. Keep existing phone-deploy/pull.

```makefile
BINARY=sni_web
OUTPUT_DIR=sni_output

.PHONY: phone-deploy phone-pull flutter-build flutter-deploy clean

phone-deploy: $(BINARY)
	adb push $(BINARY) /data/local/tmp/
	adb push GeoLite2-Country.mmdb /data/local/tmp/ 2>/dev/null || true
	adb push GeoLite2-ASN.mmdb /data/local/tmp/ 2>/dev/null || true
	adb shell "mkdir -p /data/local/tmp/$(OUTPUT_DIR)"
	adb shell "cd /data/local/tmp && SNI_OUTPUT_DIR=/data/local/tmp/$(OUTPUT_DIR) chmod +x $(BINARY) && ./$(BINARY)"
	adb forward tcp:8080 tcp:8080
	@echo "Open http://localhost:8080 in your browser"

$(BINARY):
	GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -o $(BINARY) ./cmd/sni_web/

# Flutter targets
flutter-build: $(BINARY)
	cp $(BINARY) flutter_app/assets/sni_web
	cd flutter_app && flutter build apk --release

flutter-deploy: flutter-build
	adb install flutter_app/build/app/outputs/flutter-apk/app-release.apk
	adb forward tcp:18080 tcp:18080
	@echo "APK installed. Open SNI Tester app on phone."

phone-pull:
	mkdir -p ../rust/aegis/src/resources/sni/
	adb pull /data/local/tmp/$(OUTPUT_DIR)/. ../rust/aegis/src/resources/sni/
	@echo "Results pulled. Run 'cargo build' in rust/aegis to embed."

clean:
	rm -f $(BINARY)
	rm -rf $(OUTPUT_DIR) badger_db
```

- [ ] **Step 2: Verify Makefile syntax**

```bash
cd sni_tester && make -n flutter-build
```

Expected: prints build commands without executing.

- [ ] **Step 3: Commit**

```bash
git add sni_tester/Makefile
git commit -m "ci: add flutter-build and flutter-deploy targets to Makefile"
```

---

### Task 9: Integration verification

**Files:** none (verification only)

- [ ] **Step 1: Build Go backend**

```bash
cd sni_tester && go build ./cmd/sni_web/ && go build ./cmd/sni_tester/ && go vet ./... && go test ./pkg/ -v
```

Expected: all builds pass, 6 tests pass.

- [ ] **Step 2: Build Flutter APK**

```bash
cd sni_tester && make flutter-build
```

Expected: APK generated at `flutter_app/build/app/outputs/flutter-apk/app-release.apk`.

- [ ] **Step 3: Verify complete file structure**

```bash
find sni_tester -type f -not -path '*/\.*' -not -path '*/build/*' -not -path '*/go/pkg/*' | sort
```

Expected:
- `cmd/sni_web/main.go` (no embed, no static/)
- `cmd/sni_tester/main.go` (unchanged)
- `flutter_app/` with all Flutter source files
- `pkg/` (unchanged)
- `Makefile` (updated)

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat: add Flutter frontend with Go backend API mode"
```
