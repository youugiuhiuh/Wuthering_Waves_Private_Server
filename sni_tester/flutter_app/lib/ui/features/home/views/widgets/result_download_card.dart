import 'package:flutter/material.dart';

class ResultDownloadCard extends StatelessWidget {
  final bool loading;
  final double? progress;
  final String? savedPath;
  final VoidCallback onDownload;
  final VoidCallback onOpenFolder;

  const ResultDownloadCard({
    super.key,
    this.loading = false,
    this.progress,
    this.savedPath,
    required this.onDownload,
    required this.onOpenFolder,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    if (loading || savedPath != null) {
      return Card(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('Download Results', style: theme.textTheme.titleMedium),
              const SizedBox(height: 12),
              if (loading && progress != null) ...[
                LinearProgressIndicator(value: progress),
                const SizedBox(height: 8),
                Text('Downloading... ${(progress! * 100).toStringAsFixed(0)}%'),
              ] else if (savedPath != null) ...[
                Row(
                  children: [
                    const Icon(Icons.check_circle, color: Colors.green, size: 20),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text('Saved to $savedPath',
                          style: const TextStyle(fontSize: 12)),
                    ),
                    TextButton.icon(
                      onPressed: onOpenFolder,
                      icon: const Icon(Icons.folder_open, size: 18),
                      label: const Text('Open'),
                    ),
                  ],
                ),
              ],
            ],
          ),
        ),
      );
    }
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Row(
          children: [
            Text('Results', style: theme.textTheme.titleMedium),
            const Spacer(),
            FilledButton.tonalIcon(
              onPressed: onDownload,
              icon: const Icon(Icons.download, size: 18),
              label: const Text('Download'),
            ),
          ],
        ),
      ),
    );
  }
}
