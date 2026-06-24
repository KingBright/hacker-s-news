import 'package:android_client/loop_api.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('loopPreferenceStatusLabel hides backend enum values', () {
    expect(loopPreferenceStatusLabel('processed'), '已吸收');
    expect(loopPreferenceStatusLabel('pending'), '待整理');
    expect(loopPreferenceStatusLabel('failed'), '整理失败');
    expect(loopPreferenceStatusLabel('skipped'), '已略过');
    expect(loopPreferenceStatusLabel('unknown'), isNull);
  });

  test('focusKindLabel maps focus kinds to product labels', () {
    expect(focusKindLabel('topic'), '主题');
    expect(focusKindLabel('source'), '来源');
    expect(focusKindLabel('signal'), '偏好');
    expect(focusKindLabel('format'), '形态');
  });
}
