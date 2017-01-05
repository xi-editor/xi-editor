import 'package:flutter/material.dart';
import 'package:meta/meta.dart';

/// Callback for when the FAB is pressed.
typedef void HomePageFabPressed();

/// Example [Widget] that shows a button to ping xi-core and display a
/// [message]. [HomePage] is the
class HomePage extends StatefulWidget {
  /// [HomePage] constructor.
  HomePage({
    Key key,
    this.title: 'Home Page',
    this.message: '',
    @required this.onFabPressed,
  })
      : super(key: key) {
    assert(onFabPressed != null);
  }

  /// Callback for when the [FloatingActionButton] child [Widget] is pressed.
  final HomePageFabPressed onFabPressed;

  /// A message to display in the UI.
  final String message;

  /// A title to display in the UI.
  final String title;

  @override
  _HomePageState createState() => new _HomePageState();
}

class _HomePageState extends State<HomePage> {
  int counter = 0;
  void handleFabPressed() {
    setState(() {
      counter++;
      config.onFabPressed();
    });
  }

  @override
  Widget build(BuildContext context) {
    return new Scaffold(
      appBar: new AppBar(
        title: new Text(config.title),
      ),
      body: new Center(
        child: new Text(
          'Button tapped $counter time${ counter == 1 ? '' : 's' }. \n'
              'Message: ${config.message}',
        ),
      ),
      floatingActionButton: new FloatingActionButton(
        onPressed: handleFabPressed,
        tooltip: 'Ping xi-core',
        child: new Icon(Icons.refresh),
      ),
    );
  }
}
