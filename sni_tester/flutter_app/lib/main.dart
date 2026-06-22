import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'data/services/api_client.dart';
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
