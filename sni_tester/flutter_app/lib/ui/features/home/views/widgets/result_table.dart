import 'package:flutter/material.dart';

import '../../../../data/models/models.dart';

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
            child: Text(
              'No results yet',
              style: TextStyle(color: theme.colorScheme.outline),
            ),
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
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: DataTable(
                columnSpacing: 12,
                dataRowMinHeight: 32,
                dataRowMaxHeight: 40,
                headingRowHeight: 36,
                columns: const [
                  DataColumn(label: Text('Server', style: TextStyle(fontWeight: FontWeight.w600))),
                  DataColumn(label: Text('Domain', style: TextStyle(fontWeight: FontWeight.w600))),
                  DataColumn(label: Text('Result', style: TextStyle(fontWeight: FontWeight.w600))),
                  DataColumn(label: Text('RT', style: TextStyle(fontWeight: FontWeight.w600)), numeric: true),
                  DataColumn(label: Text('Country', style: TextStyle(fontWeight: FontWeight.w600))),
                ],
                rows: results.take(100).map((r) {
                  final ok = r.success;
                  return DataRow(cells: [
                    DataCell(Text(r.server, style: const TextStyle(fontSize: 12))),
                    DataCell(Text(r.domain, style: const TextStyle(fontSize: 12))),
                    DataCell(Icon(
                      ok ? Icons.check_circle : Icons.cancel,
                      color: ok ? Colors.green : Colors.red,
                      size: 16,
                    )),
                    DataCell(Text(
                      r.responseMs != null ? '${r.responseMs!.toStringAsFixed(0)}ms' : '-',
                      style: const TextStyle(fontSize: 12),
                    )),
                    DataCell(Text(r.country ?? '-', style: const TextStyle(fontSize: 12))),
                  ]);
                }).toList(),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
