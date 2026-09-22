import 'package:flutter/material.dart';

class FreshLoopBrandMark extends StatelessWidget {
  final double size;

  const FreshLoopBrandMark({super.key, this.size = 40});

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: 'FreshLoop',
      image: true,
      child: Container(
        width: size,
        height: size,
        padding: EdgeInsets.all(size * 0.13),
        decoration: BoxDecoration(
          color: Colors.white.withValues(alpha: 0.05),
          borderRadius: BorderRadius.circular(size * 0.28),
          border: Border.all(color: Colors.white.withValues(alpha: 0.08)),
        ),
        child: Image.asset(
          'assets/brand/freshloop-mark.png',
          fit: BoxFit.contain,
          filterQuality: FilterQuality.high,
        ),
      ),
    );
  }
}
