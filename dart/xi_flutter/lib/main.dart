import 'package:flutter/material.dart';
import 'package:widgets/widgets.dart';

void main() {
  runApp(new App());
}

/// A light wrapper around [XiApp] for tracking state changes for handling UI
/// actions.
class App extends StatefulWidget {
  @override
  AppState createState() => new AppState();
}

/// State for [App].
class AppState extends State<App> {
  /// State value for holding the [message] populated as a result of UI
  /// actions triggered via [handlePingButtonPressed].
  String message;

  /// Handler passed into [XiApp] for negotiaing IPC calls to the xi-core
  /// service. Currently this is unsupported for vanilla Flutter.
  void handlePingButtonPressed() {
    setState(() {
      message = 'Pinging Xi Core is not implemented for vanilla Flutter.';
    });
  }

  @override
  Widget build(BuildContext context) {
    return new XiApp(
      message: message,
      onPingButtonPressed: handlePingButtonPressed,
    );
  }
}
