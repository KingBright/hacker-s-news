import 'package:flutter/material.dart';

class AnimatedEqualizer extends StatefulWidget {
  final Color color;
  final String size;

  const AnimatedEqualizer({
    super.key,
    this.color = const Color(0xFF19E66B),
    this.size = 'md',
  });

  @override
  State<AnimatedEqualizer> createState() => _AnimatedEqualizerState();
}

class _AnimatedEqualizerState extends State<AnimatedEqualizer> with TickerProviderStateMixin {
  late final List<AnimationController> _controllers;

  @override
  void initState() {
    super.initState();
    _controllers = List.generate(4, (index) {
      final controller = AnimationController(
        vsync: this,
        duration: Duration(milliseconds: 300 + (index * 100)),
      );
      controller.repeat(reverse: true);
      return controller;
    });
  }

  @override
  void dispose() {
    for (var controller in _controllers) {
      controller.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    double height = 20.0;
    if (widget.size == 'sm') height = 16.0;
    if (widget.size == 'lg') height = 24.0;

    return SizedBox(
      height: height,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.end,
        children: List.generate(4, (index) {
          return AnimatedBuilder(
            animation: _controllers[index],
            builder: (context, child) {
              return Container(
                margin: const EdgeInsets.symmetric(horizontal: 1.5),
                width: 3.0,
                height: height * (0.3 + 0.7 * _controllers[index].value),
                decoration: BoxDecoration(
                  color: widget.color,
                  borderRadius: BorderRadius.circular(2),
                ),
              );
            },
          );
        }),
      ),
    );
  }
}
