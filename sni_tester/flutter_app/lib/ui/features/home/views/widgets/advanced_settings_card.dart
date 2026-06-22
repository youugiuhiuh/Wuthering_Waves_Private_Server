import 'package:flutter/material.dart';

class AdvancedSettingsCard extends StatefulWidget {
  final bool aimdEnabled;
  final int fixedWorkers;
  final int ttlDays;
  final int maxLines;
  final String? geoProxy;
  final bool autoShutdown;
  final ValueChanged<bool> onAimdChanged;
  final ValueChanged<int> onFixedWorkersChanged;
  final ValueChanged<int> onTtlChanged;
  final ValueChanged<int> onMaxLinesChanged;
  final ValueChanged<String> onProxyChanged;
  final ValueChanged<bool> onAutoShutdownChanged;

  const AdvancedSettingsCard({
    super.key,
    this.aimdEnabled = true,
    this.fixedWorkers = 20,
    this.ttlDays = 7,
    this.maxLines = 0,
    this.geoProxy,
    this.autoShutdown = false,
    required this.onAimdChanged,
    required this.onFixedWorkersChanged,
    required this.onTtlChanged,
    required this.onMaxLinesChanged,
    required this.onProxyChanged,
    required this.onAutoShutdownChanged,
  });

  @override
  State<AdvancedSettingsCard> createState() => _AdvancedSettingsCardState();
}

class _AdvancedSettingsCardState extends State<AdvancedSettingsCard> {
  bool _expanded = false;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            InkWell(
              onTap: () => setState(() => _expanded = !_expanded),
              child: Row(
                children: [
                  Text('Advanced Settings', style: theme.textTheme.titleMedium),
                  const Spacer(),
                  Icon(_expanded ? Icons.expand_less : Icons.expand_more),
                ],
              ),
            ),
            if (_expanded) ...[
              const SizedBox(height: 12),
              Row(
                children: [
                  const Text('Concurrency:'),
                  const SizedBox(width: 8),
                  DropdownButton<bool>(
                    value: widget.aimdEnabled,
                    items: const [
                      DropdownMenuItem(value: true, child: Text('AIMD Auto')),
                      DropdownMenuItem(value: false, child: Text('Fixed')),
                    ],
                    onChanged: (v) {
                      if (v != null) widget.onAimdChanged(v);
                    },
                  ),
                  if (!widget.aimdEnabled) ...[
                    const SizedBox(width: 8),
                    SizedBox(
                      width: 60,
                      child: TextField(
                        decoration: const InputDecoration(isDense: true),
                        controller: TextEditingController(
                            text: widget.fixedWorkers.toString()),
                        keyboardType: TextInputType.number,
                        onChanged: (v) {
                          final n = int.tryParse(v);
                          if (n != null && n > 0) widget.onFixedWorkersChanged(n);
                        },
                      ),
                    ),
                  ],
                ],
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  const Text('TTL (days):'),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 60,
                    child: TextField(
                      decoration: const InputDecoration(isDense: true),
                      controller:
                          TextEditingController(text: widget.ttlDays.toString()),
                      keyboardType: TextInputType.number,
                      onChanged: (v) {
                        final n = int.tryParse(v);
                        if (n != null && n >= 0) widget.onTtlChanged(n);
                      },
                    ),
                  ),
                  const SizedBox(width: 16),
                  const Text('Max lines:'),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 60,
                    child: TextField(
                      decoration: const InputDecoration(
                        isDense: true,
                        hintText: 'All',
                      ),
                      controller: TextEditingController(
                          text: widget.maxLines > 0 ? widget.maxLines.toString() : ''),
                      keyboardType: TextInputType.number,
                      onChanged: (v) {
                        final n = int.tryParse(v);
                        widget.onMaxLinesChanged(n ?? 0);
                      },
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 8),
              TextField(
                decoration: const InputDecoration(
                  labelText: 'GeoIP Proxy',
                  isDense: true,
                  hintText: 'socks5://127.0.0.1:1080',
                ),
                controller: TextEditingController(text: widget.geoProxy ?? ''),
                onChanged: widget.onProxyChanged,
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  Checkbox(
                    value: widget.autoShutdown,
                    onChanged: (v) => widget.onAutoShutdownChanged(v ?? false),
                  ),
                  const Text('Auto shutdown on completion'),
                ],
              ),
            ],
          ],
        ),
      ),
    );
  }
}
