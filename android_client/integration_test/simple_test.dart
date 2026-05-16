import 'package:flutter_test/flutter_test.dart';
import 'package:android_client/main.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('Can render fallback error shell', (WidgetTester tester) async {
    await tester.pumpWidget(const ErrorApp(error: 'test error'));
    expect(find.text('App Failed to Load'), findsOneWidget);
    expect(find.textContaining('test error'), findsOneWidget);
  });
}
