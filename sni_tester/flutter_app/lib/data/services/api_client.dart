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
  final String baseUrl;
  final http.Client _http = http.Client();
  Process? _backendProcess;

  ApiClient({this.baseUrl = 'http://localhost:18080'});

  Future<void> startBackend() async {
    if (await _isRunning()) return;
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
    for (var i = 0; i < 50; i++) {
      await Future.delayed(const Duration(milliseconds: 100));
      if (await _isRunning()) return;
    }
    throw const ApiException('Backend failed to start');
  }

  Future<void> stopBackend() async {
    _backendProcess?.kill();
    await _backendProcess?.exitCode;
    _backendProcess = null;
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

  void dispose() {
    _http.close();
  }
}

class EnvKeys {
  static const outputDir = 'SNI_OUTPUT_DIR';
}
