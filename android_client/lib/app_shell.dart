import 'package:flutter/material.dart';

enum AppTab { radio, reading, loop, focus }

class LoopDraftReference {
  final String sourceType;
  final String? sourceId;
  final String? sourceUrl;
  final String? title;
  final String? quoteText;
  final int? startMs;
  final int? endMs;

  const LoopDraftReference({
    required this.sourceType,
    this.sourceId,
    this.sourceUrl,
    this.title,
    this.quoteText,
    this.startMs,
    this.endMs,
  });
}

class LoopComposeDraft {
  final String? title;
  final String? body;
  final String feedbackMode;
  final List<LoopDraftReference> references;

  const LoopComposeDraft({
    this.title,
    this.body,
    this.feedbackMode = 'balance',
    this.references = const [],
  });
}

class ShellProvider extends ChangeNotifier {
  AppTab _tab = AppTab.radio;
  LoopComposeDraft? _loopDraft;

  AppTab get tab => _tab;
  LoopComposeDraft? get loopDraft => _loopDraft;

  void selectTab(AppTab tab) {
    if (_tab == tab) return;
    _tab = tab;
    notifyListeners();
  }

  void openLoopWithDraft(LoopComposeDraft draft) {
    _loopDraft = draft;
    _tab = AppTab.loop;
    notifyListeners();
  }

  void clearLoopDraft() {
    if (_loopDraft == null) return;
    _loopDraft = null;
    notifyListeners();
  }
}
