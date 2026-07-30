import 'package:flutter/material.dart';

import '../../../../../data/models/models.dart';

class RemoteFileCard extends StatelessWidget {
  final List<FileEntry> files;
  final bool loading;
  final VoidCallback onRefresh;
  final VoidCallback onUpload;
  final ValueChanged<String> onDelete;
  final double? uploadProgress;

  const RemoteFileCard({
    super.key,
    required this.files,
    this.loading = false,
    required this.onRefresh,
    required this.onUpload,
    required this.onDelete,
    this.uploadProgress,
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
            Row(
              children: [
                Text('Phone Files', style: theme.textTheme.titleMedium),
                const Spacer(),
                if (loading)
                  const SizedBox(
                    width: 16, height: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                else
                  IconButton(
                    icon: const Icon(Icons.refresh, size: 18),
                    onPressed: onRefresh,
                    tooltip: 'Refresh',
                  ),
                IconButton(
                  icon: const Icon(Icons.cloud_upload, size: 18),
                  onPressed: onUpload,
                  tooltip: 'Upload file',
                ),
              ],
            ),
            if (uploadProgress != null) ...[
              const SizedBox(height: 8),
              LinearProgressIndicator(value: uploadProgress),
              const SizedBox(height: 4),
              Text('Uploading... ${(uploadProgress! * 100).toStringAsFixed(0)}%',
                  style: const TextStyle(fontSize: 12)),
            ],
            const SizedBox(height: 8),
            if (files.isEmpty && !loading)
              Text('No files on phone', style: TextStyle(color: theme.colorScheme.outline))
            else
              ...files.map((f) => ListTile(
                    dense: true,
                    leading: const Icon(Icons.description, size: 20),
                    title: Text(f.name, style: const TextStyle(fontSize: 13)),
                    trailing: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Text(f.sizeFormatted,
                            style: TextStyle(fontSize: 12, color: theme.colorScheme.outline)),
                        const SizedBox(width: 8),
                        InkWell(
                          onTap: () => onDelete(f.name),
                          child: Icon(Icons.delete_outline, size: 18,
                              color: theme.colorScheme.error),
                        ),
                      ],
                    ),
                  )),
          ],
        ),
      ),
    );
  }
}
