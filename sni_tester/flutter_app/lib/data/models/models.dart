class Stats {
  final int total;
  final int success;
  final int fail;
  final int timeout;
  final int error;
  final double? avgResponseMs;

  const Stats({
    this.total = 0,
    this.success = 0,
    this.fail = 0,
    this.timeout = 0,
    this.error = 0,
    this.avgResponseMs,
  });

  factory Stats.fromJson(Map<String, dynamic> json) {
    return Stats(
      total: json['total'] as int? ?? 0,
      success: json['success'] as int? ?? 0,
      fail: json['fail'] as int? ?? 0,
      timeout: json['timeout'] as int? ?? 0,
      error: json['error'] as int? ?? 0,
      avgResponseMs: (json['avg_response_ms'] as num?)?.toDouble(),
    );
  }

  Map<String, dynamic> toJson() => {
        'total': total,
        'success': success,
        'fail': fail,
        'timeout': timeout,
        'error': error,
        'avg_response_ms': avgResponseMs,
      };

  int get done => success + fail + timeout + error;
  double get progress => total > 0 ? done / total : 0.0;
}

class ProgressEvent {
  final String server;
  final String domain;
  final bool success;
  final String? errorMsg;
  final double? responseMs;
  final String? country;
  final Stats stats;

  const ProgressEvent({
    required this.server,
    required this.domain,
    required this.success,
    this.errorMsg,
    this.responseMs,
    this.country,
    required this.stats,
  });

  factory ProgressEvent.fromJson(Map<String, dynamic> json) {
    return ProgressEvent(
      server: json['server'] as String? ?? '',
      domain: json['domain'] as String? ?? '',
      success: json['success'] as bool? ?? false,
      errorMsg: json['error'] as String?,
      responseMs: (json['response_ms'] as num?)?.toDouble(),
      country: json['country'] as String?,
      stats: Stats.fromJson(json['stats'] as Map<String, dynamic>? ?? {}),
    );
  }
}

class StatusResponse {
  final String status;
  final Stats stats;
  final String? message;

  const StatusResponse({
    required this.status,
    this.stats = const Stats(),
    this.message,
  });

  factory StatusResponse.fromJson(Map<String, dynamic> json) {
    return StatusResponse(
      status: json['status'] as String? ?? 'idle',
      stats: Stats.fromJson(json['stats'] as Map<String, dynamic>? ?? {}),
      message: json['message'] as String?,
    );
  }
}

class StartParams {
  final String serversFile;
  final String domainsFile;
  final int timeoutSec;
  final int maxConcurrent;
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
}

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
