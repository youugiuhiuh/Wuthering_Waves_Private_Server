import 'package:flutter/material.dart';

import '../../../../data/models/models.dart';
import '../../../../data/services/api_client.dart';
import '../../view_models/home_view_model.dart';
import 'widgets/file_picker_card.dart';
import 'widgets/progress_section.dart';
import 'widgets/result_table.dart';
import 'widgets/settings_card.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  late final HomeViewModel _vm;

  String? _serversFile;
  String? _domainsFile;
  int _timeoutSec = 5;
  int _maxConcurrent = 20;

  @override
  void initState() {
    super.initState();
    _vm = HomeViewModel();
    _vm.addListener(_onVmChanged);
    _vm.init();
  }

  void _onVmChanged() => setState(() {});

  @override
  void dispose() {
    _vm.removeListener(_onVmChanged);
    _vm.dispose();
    super.dispose();
  }

  bool get _canStart =>
      _serversFile != null &&
      _domainsFile != null &&
      !_vm.running;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('SNI Tester'),
        actions: [
          if (_vm.running)
            IconButton(
              icon: const Icon(Icons.stop),
              tooltip: 'Stop',
              onPressed: () => _vm.stopTest(),
            ),
        ],
      ),
      body: SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: Column(
          children: [
            FilePickerCard(
              serversFile: _serversFile,
              domainsFile: _domainsFile,
              onServersPicked: (p) => setState(() => _serversFile = p),
              onDomainsPicked: (p) => setState(() => _domainsFile = p),
            ),
            const SizedBox(height: 12),
            SettingsCard(
              timeoutSec: _timeoutSec,
              maxConcurrent: _maxConcurrent,
              onTimeoutChanged: (v) => _timeoutSec = v,
              onConcurrentChanged: (v) => _maxConcurrent = v,
            ),
            const SizedBox(height: 12),
            ProgressSection(
              running: _vm.running,
              stats: _vm.stats,
              error: _vm.error,
            ),
            const SizedBox(height: 12),
            SizedBox(
              width: double.infinity,
              height: 48,
              child: FilledButton.icon(
                onPressed: _canStart
                    ? () => _vm.startTest(StartParams(
                          serversFile: _serversFile!,
                          domainsFile: _domainsFile!,
                          timeoutSec: _timeoutSec,
                          maxConcurrent: _maxConcurrent,
                        ))
                    : null,
                icon: Icon(_vm.running ? Icons.hourglass_top : Icons.play_arrow),
                label: Text(_vm.running ? 'Running...' : 'Start Test'),
              ),
            ),
            const SizedBox(height: 12),
            ResultTable(results: _vm.results),
            const SizedBox(height: 80),
          ],
        ),
      ),
    );
  }
}
