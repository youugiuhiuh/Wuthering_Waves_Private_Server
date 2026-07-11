import 'package:flutter/material.dart';
import 'package:file_picker/file_picker.dart';

import '../../../../data/models/models.dart';
import '../../../core/platform.dart';
import '../view_models/home_view_model.dart';
import 'widgets/advanced_settings_card.dart';
import 'widgets/app_mode_card.dart';
import 'widgets/connection_card.dart';
import 'widgets/file_picker_card.dart';
import 'widgets/progress_section.dart';
import 'widgets/remote_file_card.dart';
import 'widgets/result_download_card.dart';
import 'widgets/result_table.dart';
import 'widgets/settings_card.dart';

class HomeScreen extends StatefulWidget {
  final HomeViewModel viewModel;
  const HomeScreen({super.key, required this.viewModel});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  late final HomeViewModel _vm;
  String? _serversFile;
  String? _domainsFile;

  @override
  void initState() {
    super.initState();
    _vm = widget.viewModel;
    _vm.addListener(_onVmChanged);
    _vm.init();
  }

  void _onVmChanged() => setState(() {});

  @override
  void dispose() {
    _vm.removeListener(_onVmChanged);
    super.dispose();
  }

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
            // Connection / Mode card
            if (isDesktop)
              ConnectionCard(
                mode: _vm.connMode,
                connState: _vm.connState,
                deviceName: _vm.deviceName,
                errorMsg: _vm.connError,
                host: _vm.remoteHost,
                port: _vm.remotePort,
                onModeChanged: _vm.setConnMode,
                onConnect: _vm.connect,
                onDisconnect: _vm.disconnect,
                onHostChanged: _vm.setRemoteHost,
                onPortChanged: _vm.setRemotePort,
              ),
            if (isMobile)
              AppModeCard(
                mode: _vm.phoneMode,
                onModeChanged: _vm.setPhoneMode,
                backendRunning: _vm.initialized,
              ),
            const SizedBox(height: 12),

            // File selection
            if (isDesktop && _vm.connMode != ConnMode.local && _vm.isConnected)
              RemoteFileCard(
                files: _vm.remoteFiles,
                loading: _vm.filesLoading,
                onRefresh: _vm.refreshRemoteFiles,
                onUpload: _pickAndUploadFile,
                onDelete: _vm.deleteRemoteFile,
                uploadProgress: _vm.uploadProgress,
              ),
            if (isMobile || (_vm.connMode == ConnMode.local))
              FilePickerCard(
                serversFile: _serversFile,
                domainsFile: _domainsFile,
                onServersPicked: (p) => setState(() => _serversFile = p),
                onDomainsPicked: (p) => setState(() => _domainsFile = p),
              ),
            const SizedBox(height: 12),

            // Settings
            SettingsCard(
              timeoutSec: _vm.timeoutSec,
              maxConcurrent: _vm.maxConcurrent,
              dns: _vm.dns,
              debugMode: _vm.debugMode,
              forceRetest: _vm.forceRetest,
              onTimeoutChanged: (v) => _vm.updateSetting(timeoutSec: v),
              onConcurrentChanged: (v) => _vm.updateSetting(maxConcurrent: v),
              onDnsChanged: (v) => _vm.updateSetting(dns: v),
              onDebugChanged: (v) => _vm.updateSetting(debugMode: v),
              onForceRetestChanged: (v) => _vm.updateSetting(forceRetest: v),
            ),
            const SizedBox(height: 12),

            // Advanced (desktop only)
            if (isDesktop)
              AdvancedSettingsCard(
                aimdEnabled: _vm.aimdEnabled,
                fixedWorkers: _vm.fixedWorkers,
                ttlDays: _vm.ttlDays,
                maxLines: _vm.maxLines,
                geoProxy: _vm.geoProxy,
                autoShutdown: _vm.autoShutdown,
                onAimdChanged: (v) => _vm.updateSetting(aimdEnabled: v),
                onFixedWorkersChanged: (v) => _vm.updateSetting(fixedWorkers: v),
                onTtlChanged: (v) => _vm.updateSetting(ttlDays: v),
                onMaxLinesChanged: (v) => _vm.updateSetting(maxLines: v),
                onProxyChanged: (v) => _vm.updateSetting(geoProxy: v),
                onAutoShutdownChanged: (v) => _vm.updateSetting(autoShutdown: v),
              ),
            const SizedBox(height: 12),

            // Progress
            ProgressSection(
              running: _vm.running,
              stats: _vm.stats,
              error: _vm.error,
            ),
            const SizedBox(height: 12),

            // Start button
            SizedBox(
              width: double.infinity,
              height: 48,
              child: FilledButton.icon(
                onPressed: _vm.canStart
                    ? () => _vm.startTest(StartParams(
                          serversFile: _serversFile ?? '',
                          domainsFile: _domainsFile ?? '',
                          timeoutSec: _vm.timeoutSec,
                          maxConcurrent: _vm.maxConcurrent,
                        ))
                    : null,
                icon: Icon(_vm.running ? Icons.hourglass_top : Icons.play_arrow),
                label: Text(_vm.running ? 'Running...' : 'Start Test'),
              ),
            ),
            const SizedBox(height: 12),

            // Results table
            ResultTable(results: _vm.results),

            // Download (desktop only)
            if (!_vm.running && _vm.results.isNotEmpty)
              ResultDownloadCard(
                loading: _vm.downloadLoading,
                progress: _vm.downloadProgress,
                savedPath: _vm.downloadPath,
                onDownload: _vm.exportResults,
                onOpenFolder: () {},
              ),
            const SizedBox(height: 80),
          ],
        ),
      ),
    );
  }

  Future<void> _pickAndUploadFile() async {
    final result = await FilePicker.platform.pickFiles();
    if (result != null && result.files.single.path != null) {
      await _vm.uploadFile(result.files.single.path!);
    }
  }
}
