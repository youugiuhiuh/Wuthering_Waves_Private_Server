import 'dart:async';

import 'package:flutter/material.dart';

import '../../../../data/models/models.dart';
import '../../../../data/services/api_client.dart';

class HomeViewModel extends ChangeNotifier {
  final ApiClient api;

  HomeViewModel({ApiClient? api}) : api = api ?? ApiClient();

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

  Future<void> init({String? binaryPath}) async {
    if (binaryPath != null) {
      try {
        await api.startBackend(binaryPath);
      } catch (e) {
        _error = 'Backend start failed: $e';
        notifyListeners();
        return;
      }
    }
    _initialized = true;
    await refreshStatus();
  }

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

  Future<void> startTest(StartParams params) async {
    try {
      await api.startTest(params);
      _running = true;
      _results = [];
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

  @override
  void dispose() {
    _sseSub?.cancel();
    api.dispose();
    super.dispose();
  }
}
