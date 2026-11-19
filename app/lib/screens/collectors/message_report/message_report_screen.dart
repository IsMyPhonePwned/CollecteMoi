import 'dart:convert';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../../config/tenant_config.dart';
import '../../../l10n/app_localizations.dart';
import '../../../models/models.dart';
import '../../../services/upload_queue.dart';
import '../../../widgets/theme_picker_button.dart';

class MessageReportScreen extends StatefulWidget {
  const MessageReportScreen({super.key});

  @override
  State<MessageReportScreen> createState() => _MessageReportScreenState();
}

class _MessageReportScreenState extends State<MessageReportScreen> {
  final _draft = MessageReportDraft();
  final _bodyController = TextEditingController();
  final _senderController = TextEditingController();
  final _noteController = TextEditingController();
  final _urlPattern = RegExp(r'https?://[^\s]+', caseSensitive: false);
  final List<String> _attachmentNames = [];

  List<String> get _detectedUrls => _urlPattern
      .allMatches(_bodyController.text)
      .map((m) => m.group(0)!)
      .toList();

  @override
  void dispose() {
    _bodyController.dispose();
    _senderController.dispose();
    _noteController.dispose();
    super.dispose();
  }

  Future<void> _pickImages() async {
    final files = await FilePicker.pickFiles(type: FileType.image);
    if (files.isEmpty) return;
    setState(() {
      _attachmentNames.addAll(files.map((f) => f.name));
      _draft.attachmentPaths = List.of(_attachmentNames);
    });
  }

  Future<void> _submit() async {
    _draft
      ..bodyText = _bodyController.text.trim()
      ..sender = _senderController.text.trim()
      ..userNote = _noteController.text.trim()
      ..attachmentPaths = List.of(_attachmentNames);

    if (_draft.bodyText.isEmpty && _draft.attachmentPaths.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(context.tr('Add message text or a screenshot.'))),
      );
      return;
    }

    final payload = utf8.encode(
      jsonEncode({
        'channel': _draft.channel,
        'sender': _draft.sender,
        'body_text': _draft.bodyText,
        'user_note': _draft.userNote,
        'clicked_link': _draft.clickedLink,
        'replied': _draft.replied,
        'detected_urls': _detectedUrls,
        'attachments': _draft.attachmentPaths,
        'reported_at': DateTime.now().toUtc().toIso8601String(),
      }),
    );

    if (!mounted) return;
    final job = await context.read<UploadQueue>().enqueueBytes(
      artefact: ArtefactKind.messageReport,
      bytes: payload,
      label: 'message-report.json',
    );
    if (!mounted) return;
    context.push('/consent/${job.id}');
  }

  @override
  Widget build(BuildContext context) {
    final copy = context.watch<TenantConfig>().copy.messageReport;
    return Scaffold(
      appBar: AppBar(
        title: Text(context.tr(copy.title)),
        actions: const [ThemePickerButton()],
      ),
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 640),
          child: ListView(
            padding: const EdgeInsets.all(20),
            children: [
              Text(context.tr(copy.lead), style: Theme.of(context).textTheme.bodyMedium),
              const SizedBox(height: 16),
              DropdownButtonFormField<String>(
                // ignore: deprecated_member_use
                value: _draft.channel,
                decoration: InputDecoration(
                  labelText: context.tr('Channel'),
                  border: OutlineInputBorder(),
                ),
                items: const [
                  'sms',
                  'rcs',
                  'imessage',
                  'whatsapp',
                  'signal',
                  'telegram',
                  'email',
                  'social_dm',
                  'other',
                ].map((channel) {
                  final label = switch (channel) {
                    'email' => context.tr('Email'),
                    'social_dm' => context.tr('Social media message'),
                    'other' => context.tr('Other'),
                    _ => channel,
                  };
                  return DropdownMenuItem(
                    value: channel,
                    child: Text(label),
                  );
                }).toList(),
                onChanged: (v) => setState(() => _draft.channel = v ?? 'other'),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _senderController,
                decoration: InputDecoration(
                  labelText: context.tr('Sender (number, handle, address)'),
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _bodyController,
                minLines: 4,
                maxLines: 8,
                onChanged: (_) => setState(() {}),
                decoration: InputDecoration(
                  labelText: context.tr('Message text'),
                  border: OutlineInputBorder(),
                  alignLabelWithHint: true,
                ),
              ),
              if (_detectedUrls.isNotEmpty) ...[
                const SizedBox(height: 12),
                Text(
                  context.tr('Detected links (not opened):'),
                  style: Theme.of(context).textTheme.labelLarge,
                ),
                ..._detectedUrls.map(
                  (u) => ListTile(
                    dense: true,
                    leading: const Icon(Icons.link),
                    title: Text(u),
                  ),
                ),
              ],
              const SizedBox(height: 12),
              TextField(
                controller: _noteController,
                minLines: 2,
                maxLines: 4,
                decoration: InputDecoration(
                  labelText: context.tr('Why it looks suspicious'),
                  border: OutlineInputBorder(),
                ),
              ),
              SwitchListTile(
                contentPadding: EdgeInsets.zero,
                title: Text(context.tr('I clicked a link')),
                value: _draft.clickedLink,
                onChanged: (v) => setState(() => _draft.clickedLink = v),
              ),
              SwitchListTile(
                contentPadding: EdgeInsets.zero,
                title: Text(context.tr('I replied')),
                value: _draft.replied,
                onChanged: (v) => setState(() => _draft.replied = v),
              ),
              OutlinedButton.icon(
                onPressed: _pickImages,
                icon: const Icon(Icons.photo_library_outlined),
                label: Text(
                  _attachmentNames.isEmpty
                      ? context.tr('Add screenshots')
                      : context.tr('{count} screenshot(s) attached').replaceAll('{count}', '${_attachmentNames.length}'),
                ),
              ),
              const SizedBox(height: 24),
              FilledButton(onPressed: _submit, child: Text(context.tr(copy.button))),
            ],
          ),
        ),
      ),
    );
  }
}
