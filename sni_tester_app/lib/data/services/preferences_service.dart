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
