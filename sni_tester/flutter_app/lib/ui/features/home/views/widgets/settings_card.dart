import 'package:flutter/material.dart';

class SettingsCard extends StatefulWidget {
  final int timeoutSec;
  final int maxConcurrent;
  final ValueChanged<int> onTimeoutChanged;
  final ValueChanged<int> onConcurrentChanged;

  const SettingsCard({
    super.key,
    this.timeoutSec = 5,
    this.maxConcurrent = 20,
    required this.onTimeoutChanged,
    required this.onConcurrentChanged,
  });

  @override
  State<SettingsCard> createState() => _SettingsCardState();
}

class _SettingsCardState extends State<SettingsCard> {
  late TextEditingController _timeoutCtrl;
  late TextEditingController _concurrentCtrl;

  @override
  void initState() {
    super.initState();
    _timeoutCtrl = TextEditingController(text: widget.timeoutSec.toString());
    _concurrentCtrl =
        TextEditingController(text: widget.maxConcurrent.toString());
  }

  @override
  void dispose() {
    _timeoutCtrl.dispose();
    _concurrentCtrl.dispose();
    super.dispose();
  }

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
                    controller: _timeoutCtrl,
                    decoration: const InputDecoration(
                      labelText: 'Timeout (s)',
                      isDense: true,
                    ),
                    keyboardType: TextInputType.number,
                    onChanged: (v) {
                      final n = int.tryParse(v);
                      if (n != null && n > 0) widget.onTimeoutChanged(n);
                    },
                  ),
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: TextField(
                    controller: _concurrentCtrl,
                    decoration: const InputDecoration(
                      labelText: 'Max Concurrent',
                      isDense: true,
                    ),
                    keyboardType: TextInputType.number,
                    onChanged: (v) {
                      final n = int.tryParse(v);
                      if (n != null && n > 0) widget.onConcurrentChanged(n);
                    },
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
