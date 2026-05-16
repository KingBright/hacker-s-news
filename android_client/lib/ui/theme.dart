import 'package:flutter/material.dart';

class AppTheme {
  static const Color darkBackground = Color(0xFF111111);
  static const Color surfaceDark = Color(0xFF1E1E1E);
  static const Color surfaceHighlight = Color(0xFF2A2A2A);
  static const Color primaryGreen = Color(0xFF19E66B);
  static const Color textWhite = Colors.white;
  static const Color textFaded = Colors.white54;
  static const Color textMuted = Color(0xFF93C8A8);

  static ThemeData get darkTheme {
    return ThemeData.dark().copyWith(
      scaffoldBackgroundColor: darkBackground,
      primaryColor: primaryGreen,
      colorScheme: const ColorScheme.dark(
        primary: primaryGreen,
        surface: surfaceDark,
      ),
      appBarTheme: const AppBarTheme(
        backgroundColor: Colors.transparent,
        elevation: 0,
        centerTitle: false,
      ),
      bottomSheetTheme: const BottomSheetThemeData(
        backgroundColor: Colors.transparent,
      ),
      sliderTheme: SliderThemeData(
        activeTrackColor: primaryGreen,
        inactiveTrackColor: Colors.white24,
        thumbColor: primaryGreen,
        overlayColor: primaryGreen.withValues(alpha: 0.2),
      ),
    );
  }
}
