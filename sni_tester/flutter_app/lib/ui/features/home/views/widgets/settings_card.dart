import 'package:flutter/material.dart';

class SettingsCard extends StatelessWidget {
  final int timeoutSec;
  final int maxConcurrent;
  final String? dns;
  final bool debugMode;
  final bool forceRetest;
  final ValueChanged<int> onTimeoutChanged;
  final ValueChanged<int> onConcurrentChanged;
  final ValueChanged<String?> onDnsChanged;
  final ValueChanged<bool> onDebugChanged;
  final ValueChanged<bool> onForceRetestChanged;

  const SettingsCard({
    super.key,
    this.timeoutSec = 5,
    this.maxConcurrent = 20,
    this.dns,
    this.debugMode = false,
    this.forceRetest = false,
    required this.onTimeoutChanged,
    required this.onConcurrentChanged,
    required this.onDnsChanged,
    required this.onDebugChanged,
    required this.onForceRetestChanged,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Settings', style: theme.textTheme.titleMedium),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: TextField(
                    decoration: const InputDecoration(
                      labelText: 'Timeout (s)',
                      isDense: true,
                    ),
                    controller: TextEditingController(text: timeoutSec.toString()),
                    keyboardType: TextInputType.number,
                    onChanged: (v) {
                      final n = int.tryParse(v);
                      if (n != null && n > 0) onTimeoutChanged(n);
                    },
                  ),
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: TextField(
                    decoration: const InputDecoration(
                      labelText: 'Max Concurrent',
                      isDense: true,
                    ),
                    controller: TextEditingController(text: maxConcurrent.toString()),
                    keyboardType: TextInputType.number,
                    onChanged: (v) {
                      final n = int.tryParse(v);
                      if (n != null && n > 0) onConcurrentChanged(n);
                    },
                  ),
                ),
              ],
            ),
            const SizedBox(height: 12),
            DropdownButtonFormField<String>(
              initialValue: dns ?? 'auto',
              decoration: const InputDecoration(labelText: 'DNS', isDense: true),
              items: const [
                DropdownMenuItem(value: 'auto', child: Text('Auto')),
                DropdownMenuItem(value: 'doh', child: Text('DoH Only')),
                DropdownMenuItem(value: 'dot', child: Text('DoT Only')),
                DropdownMenuItem(value: 'udp', child: Text('UDP Only')),
              ],
              onChanged: (v) => onDnsChanged(v == 'auto' ? null : v),
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                Checkbox(value: debugMode, onChanged: (v) => onDebugChanged(v ?? false)),
                const Text('Debug mode'),
                const SizedBox(width: 16),
                Checkbox(value: forceRetest, onChanged: (v) => onForceRetestChanged(v ?? false)),
                const Text('Force retest'),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
