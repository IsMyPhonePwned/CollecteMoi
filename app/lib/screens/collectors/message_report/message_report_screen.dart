import 'dart:convert';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../../../config/tenant_config.dart';
import '../../../l10n/app_localizations.dart';
import '../../../models/models.dart';
import '../../../services/upload_queue.dart';
import '../../../widgets/settings_button.dart';

class MessageReportScreen extends StatefulWidget {
  const MessageReportScreen({super.key});

  @override
  State<MessageReportScreen> createState() => _MessageReportScreenState();
}

class _Attachment {
  _Attachment({required this.name, this.bytes});
  final String name;
  final Uint8List? bytes;
}

class _MessageReportScreenState extends State<MessageReportScreen> {
  final _draft = MessageReportDraft();
  final _bodyController = TextEditingController();
  final _senderController = TextEditingController();
  final _noteController = TextEditingController();
  final _urlPattern = RegExp(r'https?://[^\s]+', caseSensitive: false);
  final List<_Attachment> _attachments = [];

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
    final next = <_Attachment>[];
    for (final f in files) {
      Uint8List? bytes;
      try {
        bytes = await f.readAsBytes();
      } catch (_) {
        bytes = null;
      }
      // Prefer in-memory bytes so admin can preview screenshots from the sealed JSON.
      if (bytes == null || bytes.isEmpty) {
        try {
          bytes = await f.xFile.readAsBytes();
        } catch (_) {
          bytes = null;
        }
      }
      next.add(_Attachment(name: f.name, bytes: bytes));
    }
    final missing = next.where((a) => a.bytes == null || a.bytes!.isEmpty).length;
    setState(() {
      _attachments.addAll(next);
      _draft.attachmentPaths = _attachments.map((a) => a.name).toList();
    });
    if (!mounted) return;
    if (missing > 0) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            context
                .tr('{count} screenshot(s) could not be loaded into memory')
                .replaceAll('{count}', '$missing'),
          ),
        ),
      );
    }
  }

  void _removeAttachment(int index) {
    setState(() {
      _attachments.removeAt(index);
      _draft.attachmentPaths = _attachments.map((a) => a.name).toList();
    });
  }

  String _mimeForName(String name) {
    final lower = name.toLowerCase();
    if (lower.endsWith('.png')) return 'image/png';
    if (lower.endsWith('.gif')) return 'image/gif';
    if (lower.endsWith('.webp')) return 'image/webp';
    if (lower.endsWith('.heic') || lower.endsWith('.heif')) return 'image/heic';
    return 'image/jpeg';
  }

  Future<void> _submit() async {
    _draft
      ..bodyText = _bodyController.text.trim()
      ..sender = _senderController.text.trim()
      ..userNote = _noteController.text.trim()
      ..attachmentPaths = _attachments.map((a) => a.name).toList();

    if (_draft.bodyText.isEmpty && _draft.attachmentPaths.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(context.tr('Add message text or a screenshot.'))),
      );
      return;
    }

    final attachmentPayload = <Map<String, dynamic>>[];
    for (final a in _attachments.take(8)) {
      final entry = <String, dynamic>{'name': a.name};
      final bytes = a.bytes;
      // Keep screenshots in the sealed payload so admin can preview them.
      if (bytes != null && bytes.isNotEmpty && bytes.length <= 4 * 1024 * 1024) {
        entry['mime'] = _mimeForName(a.name);
        entry['data_base64'] = base64Encode(bytes);
      }
      attachmentPayload.add(entry);
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
        'attachments': attachmentPayload,
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
    context.watch<TenantConfig>();
    final copyTitle = context.branded((c) => c.messageReport.title);
    final copyLead = context.branded((c) => c.messageReport.lead);
    final copyButton = context.branded((c) => c.messageReport.button);

    return Scaffold(
      appBar: AppBar(
        title: Text(copyTitle),
        actions: const [SettingsButton()],
      ),
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 640),
          child: ListView(
            padding: const EdgeInsets.all(20),
            children: [
              Text(copyLead, style: Theme.of(context).textTheme.bodyMedium),
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
                onChanged: (_) => setState(() {}),
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
                onChanged: (_) => setState(() {}),
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
                  _attachments.isEmpty
                      ? context.tr('Add screenshots')
                      : context.tr('{count} screenshot(s) attached').replaceAll(
                            '{count}',
                            '${_attachments.length}',
                          ),
                ),
              ),
              if (_attachments.isNotEmpty) ...[
                const SizedBox(height: 12),
                SizedBox(
                  height: 96,
                  child: ListView.separated(
                    scrollDirection: Axis.horizontal,
                    itemCount: _attachments.length,
                    separatorBuilder: (_, _) => const SizedBox(width: 8),
                    itemBuilder: (context, i) {
                      final att = _attachments[i];
                      return Stack(
                        children: [
                          ClipRRect(
                            borderRadius: BorderRadius.circular(10),
                            child: AspectRatio(
                              aspectRatio: 1,
                              child: att.bytes != null
                                  ? Image.memory(
                                      att.bytes!,
                                      fit: BoxFit.cover,
                                    )
                                  : ColoredBox(
                                      color: Theme.of(context)
                                          .colorScheme
                                          .surfaceContainerHighest,
                                      child: Center(
                                        child: Padding(
                                          padding: const EdgeInsets.all(6),
                                          child: Text(
                                            att.name,
                                            maxLines: 3,
                                            overflow: TextOverflow.ellipsis,
                                            textAlign: TextAlign.center,
                                            style: Theme.of(context)
                                                .textTheme
                                                .labelSmall,
                                          ),
                                        ),
                                      ),
                                    ),
                            ),
                          ),
                          Positioned(
                            top: 2,
                            right: 2,
                            child: Material(
                              color: Colors.black54,
                              shape: const CircleBorder(),
                              child: InkWell(
                                customBorder: const CircleBorder(),
                                onTap: () => _removeAttachment(i),
                                child: const Padding(
                                  padding: EdgeInsets.all(4),
                                  child: Icon(
                                    Icons.close,
                                    size: 16,
                                    color: Colors.white,
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ],
                      );
                    },
                  ),
                ),
              ],
              const SizedBox(height: 24),
              FilledButton(onPressed: _submit, child: Text(copyButton)),
            ],
          ),
        ),
      ),
    );
  }
}
