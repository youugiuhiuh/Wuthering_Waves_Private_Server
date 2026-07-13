import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart' hide ConnectionState;
import 'package:path_provider/path_provider.dart';

import '../../../../data/models/models.dart';
import '../../../../data/services/api_client.dart';
import '../../../../data/services/notification_service.dart';
import '../../../../data/services/preferences_service.dart';
import '../views/widgets/app_mode_card.dart';

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
  Timer? _notifyTimer;
  bool _notifyPending = false;
  static const _maxResults = 50;

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
    if (_connMode == ConnMode.local || _phoneMode == PhoneMode.local) {
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
    if (mode == PhoneMode.local) {
      _startLocalBackend();
    } else {
      _setDisconnected();
    }
    notifyListeners();
  }

  Future<void> _startLocalBackend() async {
    _connState = ConnectionState.connecting;
    notifyListeners();
    try {
      await api.startBackend();
      _setConnected('Local');
      await refreshStatus();
    } catch (e) {
      _connState = ConnectionState.error;
      _connError = e.toString();
      notifyListeners();
    }
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
      NotificationService.start();
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
        _results.insert(0, event);
        if (_results.length > _maxResults) _results.removeLast();
        NotificationService.updateProgress(_stats.done, _stats.total);
        if (_stats.done >= _stats.total && _stats.total > 0) {
          _running = false;
          NotificationService.complete(_stats.success, _stats.fail);
          _notifyTimer?.cancel();
          _notifyPending = false;
          notifyListeners();
          return;
        }
        _error = null;
        _scheduleNotify();
      },
      onError: (e) {
        _error = 'SSE: $e';
        notifyListeners();
      },
    );
  }

  void _scheduleNotify() {
    if (_notifyPending) return;
    _notifyPending = true;
    _notifyTimer?.cancel();
    _notifyTimer = Timer(const Duration(milliseconds: 100), () {
      _notifyPending = false;
      notifyListeners();
    });
  }

  // --- Download ---
  Future<void> downloadResults() async {
    _downloadLoading = true;
    _downloadProgress = null;
    notifyListeners();
    try {
      final dir = Directory(
          '${(await getApplicationDocumentsDirectory()).path}/sni_tester_results');
      if (!dir.existsSync()) dir.createSync(recursive: true);
      final ts = DateTime.now().millisecondsSinceEpoch;
      final path = '${dir.path}/results_$ts.pb';
      await api.downloadResult(path);
      _downloadPath = path;
    } catch (e) {
      _error = 'Download failed: $e';
    }
    _downloadLoading = false;
    notifyListeners();
  }

  Future<void> exportResults() async {
    _downloadLoading = true;
    _downloadProgress = null;
    notifyListeners();
    try {
      final dir = Directory('/storage/emulated/0/Download');
      if (!await dir.exists()) {
        await dir.create(recursive: true);
      }
      final ts = DateTime.now().millisecondsSinceEpoch;
      final path = '${dir.path}/sni_results_$ts.zip';
      await api.downloadResult(path);
      _downloadPath = path;
    } catch (e) {
      _error = 'Export failed: $e';
    }
    _downloadLoading = false;
    notifyListeners();
  }

  @override
  void dispose() {
    _notifyTimer?.cancel();
    _sseSub?.cancel();
    api.dispose();
    super.dispose();
  }
}
