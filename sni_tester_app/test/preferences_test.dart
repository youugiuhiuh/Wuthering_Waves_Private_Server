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
