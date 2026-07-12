import 'package:flutter/material.dart';

import '../../../../../data/models/models.dart';

class ResultTable extends StatelessWidget {
  final List<ProgressEvent> results;

  const ResultTable({super.key, required this.results});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    if (results.isEmpty) {
      return Card(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Center(
            child: Text('No results yet',
                style: TextStyle(color: theme.colorScheme.outline)),
          ),
        ),
      );
    }
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Results (${results.length})',
                style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            SizedBox(
              height: 280,
              child: ListView.builder(
                itemCount: results.length,
                itemExtent: 28,
                itemBuilder: (_, i) => _ResultRow(event: results[i]),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ResultRow extends StatelessWidget {
  final ProgressEvent event;
  const _ResultRow({required this.event});

  @override
  Widget build(BuildContext context) {
    final ok = event.success;
    return Row(
      children: [
        Icon(
          ok ? Icons.check_circle : Icons.cancel,
          color: ok ? Colors.green : Colors.red,
          size: 16,
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Text(
            event.domain,
            style: const TextStyle(fontSize: 12),
            overflow: TextOverflow.ellipsis,
          ),
        ),
        const SizedBox(width: 8),
        if (event.country != null && event.country!.isNotEmpty)
          Text(event.country!, style: const TextStyle(fontSize: 12, color: Colors.grey)),
        if (event.responseMs != null) ...[
          const SizedBox(width: 4),
          Text(
            '${event.responseMs!.toStringAsFixed(0)}ms',
            style: const TextStyle(fontSize: 11, color: Colors.grey),
          ),
        ],
      ],
    );
  }
}
