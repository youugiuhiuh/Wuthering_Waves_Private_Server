import 'package:flutter/material.dart' hide ConnectionState;

import '../../../../../data/models/models.dart';

class ConnectionCard extends StatelessWidget {
  final ConnMode mode;
  final ConnectionState connState;
  final String? deviceName;
  final String? errorMsg;
  final String host;
  final int port;
  final ValueChanged<ConnMode> onModeChanged;
  final VoidCallback onConnect;
  final VoidCallback onDisconnect;
  final ValueChanged<String> onHostChanged;
  final ValueChanged<int> onPortChanged;

  const ConnectionCard({
    super.key,
    required this.mode,
    required this.connState,
    this.deviceName,
    this.errorMsg,
    this.host = 'localhost',
    this.port = 18080,
    required this.onModeChanged,
    required this.onConnect,
    required this.onDisconnect,
    required this.onHostChanged,
    required this.onPortChanged,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isConnected = connState == ConnectionState.connected;
    final isConnecting = connState == ConnectionState.connecting;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Connection', style: theme.textTheme.titleMedium),
            const SizedBox(height: 12),
            SegmentedButton<ConnMode>(
              segments: const [
                ButtonSegment(value: ConnMode.local, label: Text('Local'), icon: Icon(Icons.computer)),
                ButtonSegment(value: ConnMode.usb, label: Text('USB'), icon: Icon(Icons.usb)),
                ButtonSegment(value: ConnMode.wifi, label: Text('WiFi'), icon: Icon(Icons.wifi)),
              ],
              selected: {mode},
              onSelectionChanged: (s) => onModeChanged(s.first),
            ),
            if (mode == ConnMode.wifi) ...[
              const SizedBox(height: 12),
              Row(
                children: [
                  Expanded(
                    child: TextField(
                      decoration: const InputDecoration(
                        labelText: 'Host',
                        isDense: true,
                        hintText: '192.168.1.100',
                      ),
                      controller: TextEditingController(text: host),
                      onChanged: onHostChanged,
                    ),
                  ),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 80,
                    child: TextField(
                      decoration: const InputDecoration(
                        labelText: 'Port',
                        isDense: true,
                      ),
                      controller: TextEditingController(text: port.toString()),
                      keyboardType: TextInputType.number,
                      onChanged: (v) {
                        final n = int.tryParse(v);
                        if (n != null) onPortChanged(n);
                      },
                    ),
                  ),
                ],
              ),
            ],
            const SizedBox(height: 12),
            Row(
              children: [
                _statusIndicator(context),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    isConnected
                        ? 'Connected ${deviceName != null ? 'to $deviceName' : ''}'
                        : isConnecting
                            ? 'Connecting...'
                            : errorMsg ?? 'Not connected',
                    style: TextStyle(
                      color: isConnected
                          ? Colors.green
                          : isConnecting
                              ? theme.colorScheme.primary
                              : errorMsg != null
                                  ? theme.colorScheme.error
                                  : theme.colorScheme.outline,
                    ),
                  ),
                ),
                if (isConnected)
                  TextButton(onPressed: onDisconnect, child: const Text('Disconnect'))
                else
                  FilledButton.tonal(
                    onPressed: isConnecting ? null : onConnect,
                    child: Text(isConnecting ? '...' : 'Connect'),
                  ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  Widget _statusIndicator(BuildContext context) {
    final color = switch (connState) {
      ConnectionState.connected => Colors.green,
      ConnectionState.connecting => Theme.of(context).colorScheme.primary,
      ConnectionState.error => Theme.of(context).colorScheme.error,
      ConnectionState.disconnected => Theme.of(context).colorScheme.outline,
    };
    return Container(
      width: 12,
      height: 12,
      decoration: BoxDecoration(
        color: color,
        shape: BoxShape.circle,
      ),
    );
  }
}
