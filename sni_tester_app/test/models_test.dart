import 'package:flutter_test/flutter_test.dart';

import 'package:sni_tester/data/models/models.dart';

void main() {
  group('Stats', () {
    test('default values', () {
      final s = Stats();
      expect(s.total, 0);
      expect(s.done, 0);
      expect(s.progress, 0.0);
    });

    test('fromJson parses all fields', () {
      final j = {
        'total': 100,
        'success': 80,
        'fail': 10,
        'timeout': 5,
        'error': 5,
        'avg_response_ms': 42.5,
      };
      final s = Stats.fromJson(j);
      expect(s.total, 100);
      expect(s.success, 80);
      expect(s.fail, 10);
      expect(s.timeout, 5);
      expect(s.error, 5);
      expect(s.avgResponseMs, 42.5);
    });

    test('fromJson handles null', () {
      final s = Stats.fromJson({});
      expect(s.total, 0);
      expect(s.avgResponseMs, null);
    });

    test('progress calculation', () {
      final s = Stats(total: 100, success: 50, fail: 25);
      expect(s.done, 75);
      expect(s.progress, 0.75);
    });
  });

  group('ProgressEvent', () {
    test('fromJson parses success event', () {
      final j = {
        'server': 'sni.example.com',
        'domain': 'test.com',
        'success': true,
        'response_ms': 123.4,
        'country': 'US',
        'stats': {'total': 10, 'success': 5, 'fail': 0, 'timeout': 0, 'error': 0},
      };
      final e = ProgressEvent.fromJson(j);
      expect(e.server, 'sni.example.com');
      expect(e.domain, 'test.com');
      expect(e.success, true);
      expect(e.responseMs, 123.4);
      expect(e.country, 'US');
      expect(e.stats.total, 10);
    });

    test('fromJson handles error event', () {
      final j = {
        'server': 'sni.example.com',
        'domain': 'bad.com',
        'success': false,
        'error': 'Connection refused',
        'stats': {'total': 10, 'success': 5, 'fail': 1, 'timeout': 0, 'error': 0},
      };
      final e = ProgressEvent.fromJson(j);
      expect(e.success, false);
      expect(e.errorMsg, 'Connection refused');
    });
  });

  group('StatusResponse', () {
    test('fromJson parses status', () {
      final j = {
        'status': 'running',
        'stats': {'total': 50, 'success': 20, 'fail': 0, 'timeout': 0, 'error': 0},
        'message': 'Processing...',
      };
      final s = StatusResponse.fromJson(j);
      expect(s.status, 'running');
      expect(s.stats.total, 50);
      expect(s.message, 'Processing...');
    });

    test('default status is idle', () {
      final s = StatusResponse.fromJson({});
      expect(s.status, 'idle');
    });
  });

  group('StartParams', () {
    test('toJson produces correct map', () {
      final p = StartParams(
        serversFile: '/tmp/servers.txt',
        domainsFile: '/tmp/domains.txt',
        timeoutSec: 10,
        maxConcurrent: 50,
      );
      final j = p.toJson();
      expect(j['servers_file'], '/tmp/servers.txt');
      expect(j['domains_file'], '/tmp/domains.txt');
      expect(j['timeout_sec'], 10);
      expect(j['max_concurrent'], 50);
    });
  });

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
}
