import 'package:flutter/material.dart';

import '../../../../../data/models/models.dart';

class ProgressSection extends StatelessWidget {
  final bool running;
  final Stats stats;
  final String? error;

  const ProgressSection({
    super.key,
    required this.running,
    required this.stats,
    this.error,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final progress = stats.progress;
    final color = error != null
        ? theme.colorScheme.error
        : running
            ? theme.colorScheme.primary
            : stats.done > 0
                ? theme.colorScheme.tertiary
                : theme.colorScheme.outline;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Text('Progress', style: theme.textTheme.titleMedium),
                const Spacer(),
                if (running)
                  SizedBox(
                    width: 16,
                    height: 16,
                    child: CircularProgressIndicator(
                      strokeWidth: 2,
                      color: theme.colorScheme.primary,
                    ),
                  ),
              ],
            ),
            const SizedBox(height: 12),
            ClipRRect(
              borderRadius: BorderRadius.circular(4),
              child: LinearProgressIndicator(
                value: progress,
                minHeight: 8,
                backgroundColor: theme.colorScheme.surfaceContainerHighest,
              ),
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                _statChip(color, '${stats.done}/${stats.total}',
                    stats.total > 0 ? '${(progress * 100).toStringAsFixed(0)}%' : ''),
                const SizedBox(width: 8),
                _statChip(Colors.green, '${stats.success}', 'OK'),
                const SizedBox(width: 8),
                _statChip(Colors.red, '${stats.fail}', 'Fail'),
                const SizedBox(width: 8),
                _statChip(Colors.orange, '${stats.timeout}', 'Timeout'),
                const SizedBox(width: 8),
                _statChip(Colors.grey, '${stats.error}', 'Err'),
              ],
            ),
            if (error != null) ...[
              const SizedBox(height: 8),
              Text(error!, style: TextStyle(color: theme.colorScheme.error, fontSize: 12)),
            ],
          ],
        ),
      ),
    );
  }

  Widget _statChip(Color color, String value, String label) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.15),
        borderRadius: BorderRadius.circular(12),
      ),
      child: Text('$value $label',
          style: TextStyle(color: color, fontSize: 12, fontWeight: FontWeight.w600)),
    );
  }
}
