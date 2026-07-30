import 'package:flutter/material.dart';

enum PhoneMode { local, controlled }

class AppModeCard extends StatelessWidget {
  final PhoneMode mode;
  final ValueChanged<PhoneMode> onModeChanged;
  final bool backendRunning;

  const AppModeCard({
    super.key,
    required this.mode,
    required this.onModeChanged,
    this.backendRunning = false,
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
            Text('Mode', style: theme.textTheme.titleMedium),
            const SizedBox(height: 12),
            SegmentedButton<PhoneMode>(
              segments: const [
                ButtonSegment(
                  value: PhoneMode.local,
                  label: Text('Local'),
                  icon: Icon(Icons.phone_android),
                ),
                ButtonSegment(
                  value: PhoneMode.controlled,
                  label: Text('Controlled'),
                  icon: Icon(Icons.cast),
                ),
              ],
              selected: {mode},
              onSelectionChanged: (s) => onModeChanged(s.first),
            ),
            if (mode == PhoneMode.controlled) ...[
              const SizedBox(height: 16),
              Row(
                children: [
                  Icon(Icons.wifi, color: backendRunning ? Colors.green : theme.colorScheme.outline),
                  const SizedBox(width: 8),
                  Text(
                    backendRunning ? 'Port 18080 — Waiting for desktop...' : 'Backend not running',
                    style: TextStyle(
                      color: backendRunning ? null : theme.colorScheme.error,
                    ),
                  ),
                ],
              ),
            ],
          ],
        ),
      ),
    );
  }
}
