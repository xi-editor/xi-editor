import 'package:flutter/material.dart';
import 'package:meta/meta.dart';

import 'home_page.dart';

/// Exmple top-level [Widget] based on scaffolding from the `flutter create`
/// command.
class XiApp extends StatelessWidget {
  /// Maps [XiApp]'s [onPingButtonPressed] method to the [HomePage]'s FAB
  /// button press.
  final HomePageFabPressed onPingButtonPressed;

  /// Allows parent [Widget]s in either vanilla Flutter or Fuchsia to modify
  /// the [HomePage]'s [message].
  final String message;

  /// [XiApp] constructor.
  XiApp({
    Key key,
    this.message,
    @required this.onPingButtonPressed,
  })
      : super(key: key) {
    assert(onPingButtonPressed != null);
  }

  /// Uses a [MaterialApp] as the root of the Xi UI hierarchy.
  @override
  Widget build(BuildContext context) {
    return new MaterialApp(
      title: 'Xi',
      theme: new ThemeData(
        primarySwatch: Colors.blue,
      ),
      home: new HomePage(
        title: 'Xi Example Home Page',
        message: message,
        onFabPressed: onPingButtonPressed,
      ),
    );
  }
}
