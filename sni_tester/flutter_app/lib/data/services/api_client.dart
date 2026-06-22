import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:http/http.dart' as http;
import 'package:path_provider/path_provider.dart';

import '../models/models.dart';

class ApiException implements Exception {
  final String message;
  const ApiException(this.message);
  @override
  String toString() => message;
}

class ApiClient {
  String baseUrl;
  final http.Client _http = http.Client();
  Process? _backendProcess;
  bool _hasStartedLocal = false;

  ApiClient({this.baseUrl = 'http://localhost:18080'});

  bool get isRemote => !baseUrl.contains('localhost') && !baseUrl.contains('127.0.0.1');

  void updateUrl(String url) {
    baseUrl = url;
  }

  Future<void> startBackend() async {
    if (await _isRunning()) return;
    if (isRemote) return;
    await _startLocalBinary();
  }

  Future<void> _startLocalBinary() async {
    final dir = await getApplicationDocumentsDirectory();
    final binaryPath = '${dir.path}/sni_web';
    final file = File(binaryPath);
    if (!file.existsSync()) {
      final data = await rootBundle.load('assets/sni_web');
      await file.writeAsBytes(data.buffer.asUint8List());
      await Process.run('chmod', ['+x', binaryPath]);
    }
    _backendProcess = await Process.start(binaryPath, [],
        environment: {EnvKeys.outputDir: '${dir.path}/sni_output'});
    _backendProcess!.stderr
        .transform(utf8.decoder)
        .listen((line) => stderr.writeln('[sni_web] $line'));
    _hasStartedLocal = true;
    for (var i = 0; i < 50; i++) {
      await Future.delayed(const Duration(milliseconds: 100));
      if (await _isRunning()) return;
    }
    throw const ApiException('Backend failed to start');
  }

  Future<void> stopBackend() async {
    if (_hasStartedLocal) {
      _backendProcess?.kill();
      await _backendProcess?.exitCode;
      _backendProcess = null;
      _hasStartedLocal = false;
    }
  }

  Future<bool> _isRunning() async {
    try {
      final res = await _http
          .get(Uri.parse('$baseUrl/api/health'))
          .timeout(const Duration(seconds: 2));
      return res.statusCode == 200;
    } catch (_) {
      return false;
    }
  }

  Future<StatusResponse> getStatus() async {
    final res = await _http
        .get(Uri.parse('$baseUrl/api/status'))
        .timeout(const Duration(seconds: 5));
    if (res.statusCode != 200) {
      throw ApiException('Status failed: ${res.statusCode}');
    }
    return StatusResponse.fromJson(
        jsonDecode(res.body) as Map<String, dynamic>);
  }

  Future<void> startTest(StartParams params) async {
    final res = await _http
        .post(
          Uri.parse('$baseUrl/api/start'),
          headers: {'Content-Type': 'application/json'},
          body: jsonEncode(params.toJson()),
        )
        .timeout(const Duration(seconds: 10));
    if (res.statusCode != 200) {
      throw ApiException('Start failed: ${res.statusCode} ${res.body}');
    }
  }

  Future<void> stopTest() async {
    try {
      await _http
          .post(Uri.parse('$baseUrl/api/stop'))
          .timeout(const Duration(seconds: 5));
    } catch (_) {}
  }

  Stream<ProgressEvent> progressStream() {
    late StreamController<ProgressEvent> controller;
    final client = http.Client();
    controller = StreamController<ProgressEvent>(
      onCancel: () => client.close(),
    );

    _fetchSSE(client, controller);

    return controller.stream;
  }

  Future<void> _fetchSSE(
      http.Client client, StreamController<ProgressEvent> controller) async {
    try {
      final request =
          http.Request('GET', Uri.parse('$baseUrl/api/progress'));
      final response = await client.send(request);
      response.stream
          .transform(utf8.decoder)
          .transform(const LineSplitter())
          .listen(
        (line) {
          if (!line.startsWith('data:')) return;
          final data = line.substring(5).trim();
          if (data.isEmpty) return;
          try {
            final json = jsonDecode(data) as Map<String, dynamic>;
            controller.add(ProgressEvent.fromJson(json));
          } catch (e) {
            stderr.writeln('SSE parse error: $e');
          }
        },
        onError: (e) {
          controller.addError(ApiException('SSE error: $e'));
          client.close();
        },
        onDone: () => client.close(),
      );
    } catch (e) {
      controller.addError(ApiException('SSE connection: $e'));
      client.close();
    }
  }

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

  void dispose() {
    _http.close();
  }
}

class EnvKeys {
  static const outputDir = 'SNI_OUTPUT_DIR';
}
