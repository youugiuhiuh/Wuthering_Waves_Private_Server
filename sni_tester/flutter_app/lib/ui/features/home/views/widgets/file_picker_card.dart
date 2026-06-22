import 'dart:io';

import 'package:flutter/material.dart';
import 'package:file_picker/file_picker.dart';

class FilePickerCard extends StatelessWidget {
  final String? serversFile;
  final String? domainsFile;
  final ValueChanged<String> onServersPicked;
  final ValueChanged<String> onDomainsPicked;

  const FilePickerCard({
    super.key,
    this.serversFile,
    this.domainsFile,
    required this.onServersPicked,
    required this.onDomainsPicked,
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
            Text('Input Files', style: theme.textTheme.titleMedium),
            const SizedBox(height: 12),
            _fileRow(
              context,
              label: 'Servers File',
              value: serversFile != null
                  ? serversFile!.split(Platform.pathSeparator).last
                  : null,
              onPick: () => _pickFile(onServersPicked),
            ),
            const SizedBox(height: 8),
            _fileRow(
              context,
              label: 'Domains File',
              value: domainsFile != null
                  ? domainsFile!.split(Platform.pathSeparator).last
                  : null,
              onPick: () => _pickFile(onDomainsPicked),
            ),
          ],
        ),
      ),
    );
  }

  Widget _fileRow(
    BuildContext context, {
    required String label,
    String? value,
    required VoidCallback onPick,
  }) {
    return Row(
      children: [
        SizedBox(
          width: 120,
          child: Text(label, style: const TextStyle(fontWeight: FontWeight.w500)),
        ),
        Expanded(
          child:         Text(
            value ?? 'Not selected',
            style: TextStyle(
              color: value == null ? Theme.of(context).colorScheme.outline : null,
              fontStyle: value == null ? FontStyle.italic : FontStyle.normal,
            ),
            overflow: TextOverflow.ellipsis,
          ),
        ),
        const SizedBox(width: 8),
        FilledButton.tonalIcon(
          onPressed: onPick,
          icon: const Icon(Icons.folder_open, size: 18),
          label: const Text('Browse'),
        ),
      ],
    );
  }

  Future<void> _pickFile(ValueChanged<String> onPicked) async {
    final result = await FilePicker.platform.pickFiles();
    if (result != null && result.files.single.path != null) {
      onPicked(result.files.single.path!);
    }
  }
}
