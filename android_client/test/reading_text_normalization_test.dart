import 'package:android_client/ui/reading_screen.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'normalizeReaderParagraphText keeps punctuation instead of literal capture markers',
    () {
      final normalized = normalizeReaderParagraphText(
        'Hello , world 。 中文 ， 内容 （ test ）',
      );

      expect(normalized, isNot(contains(r'$1')));
      expect(normalized, contains('Hello, world。'));
      expect(normalized, contains('中文，'));
      expect(normalized, contains('（test）'));
    },
  );

  test(
    'normalizeReaderParagraphText removes orphan dollar markers but keeps money',
    () {
      final normalized = normalizeReaderParagraphText(
        r'这里$1有残片，括号（$2）。价格 $148,337、$1.99 和 $1 per month 保留。',
      );

      expect(normalized, contains('这里有残片'));
      expect(normalized, contains('括号（）。'));
      expect(normalized, contains(r'$148,337'));
      expect(normalized, contains(r'$1.99'));
      expect(normalized, contains(r'$1 per month'));
      expect(normalized, isNot(contains(r'这里$1')));
    },
  );
}
