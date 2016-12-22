import 'package:apps.modular.lib.app.dart/app.dart';
import 'package:apps.modular.services.application/service_provider.fidl.dart';
import 'package:apps.modular.services.application/application_launcher.fidl.dart';
import 'package:apps.modular.services.document_store/document.fidl.dart';
import 'package:apps.modular.services.story/link.fidl.dart';
import 'package:apps.modular.services.story/module.fidl.dart';
import 'package:apps.modular.services.story/module_controller.fidl.dart';
import 'package:apps.modular.services.story/story.fidl.dart';
import 'package:apps.mozart.lib.flutter/child_view.dart';
import 'package:apps.mozart.services.views/view_token.fidl.dart';
import 'package:apps.xi.services/xi.fidl.dart';
import 'package:flutter/material.dart';
import 'package:widgets/widgets.dart';
import 'package:lib.fidl.dart/bindings.dart';
import 'package:lib.fidl.dart/core.dart' as core;
import 'dart:typed_data';
import 'dart:convert';

import 'src/peer.dart';

final ApplicationContext _context = new ApplicationContext.fromStartupInfo();

const String _kXiCoreURL = 'file:///system/apps/xi-core';

ModuleImpl _module;

void _log(String msg) {
  print('[xi_app] $msg');
}

/// An implementation of the [Module] interface.
class ModuleImpl extends Module {
  final ModuleBinding _binding = new ModuleBinding();
  final StoryProxy _story = new StoryProxy();
  final LinkProxy _link = new LinkProxy();

  /// Bind an [InterfaceRequest] for a [Module] interface to this object.
  void bind(InterfaceRequest<Module> request) {
    _binding.bind(this, request);
  }

  @override
  void initialize(
      InterfaceHandle<Story> storyHandle,
      InterfaceHandle<Link> linkHandle,
      InterfaceHandle<ServiceProvider> incomingServices,
      InterfaceRequest<ServiceProvider> outgoingServices) {
    _log('ModuleImpl::initialize call');

    _story.ctrl.bind(storyHandle);
    _link.ctrl.bind(linkHandle);
  }

  @override
  void stop(void callback()) {
    _log('ModuleImpl::stop call');

    // Cleaning up.
    _link.ctrl.close();
    _story.ctrl.close();

    // Invoke the callback to signal that the clean-up process is done.
    callback();
  }
}

/// A light wrapper around [XiApp] for tracking state changes for handling UI
/// actions.
class App extends StatefulWidget {
  @override
  AppState createState() => new AppState();
}

/// State for [App].
class AppState extends State<App> {
  final ServiceProviderProxy _serviceProvider = new ServiceProviderProxy();
  final ApplicationLaunchInfo _launchInfo = new ApplicationLaunchInfo();
  final XiPeer _xi = new XiPeer();

  /// State value for holding the [message] populated as a result of UI
  /// actions triggered via [handlePingButtonPressed].
  String message;

  @override
  void initState() {
    _launchInfo.url = _kXiCoreURL;
    _launchInfo.services = _serviceProvider.ctrl.request();
    _context.launcher.createApplication(_launchInfo, null);
    _xi.bind(connectToServiceByName(_serviceProvider, 'xi.Json'));
    _xi.onRead = _handleXiRead;
    super.initState();
  }

  void _handleXiRead(String data) {
    setState(() => message = "got string $data");
  }

  /// Handler passed into [XiApp] for negotiaing IPC calls to the xi-core
  /// service. Currently this is unsupported for vanilla Flutter.
  void handlePingButtonPressed() {
    setState(() {
      message = 'Sending request...';
      _log(message);
      _xi.send("{\"method\": \"new_tab\", \"params\": \"[]\", \"id\": 1}\n");
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

/// Main entry point to the example parent module.
void main() {
  _log('Module main called');

  _context.outgoingServices.addServiceForName(
    (InterfaceRequest<Module> request) {
      _log('Received binding request for Module');

      _module = new ModuleImpl()..bind(request);

      if (_module != null) {
        _log('Module interface can only be provided once. '
            'Rejecting request.');
        request.channel.close();
        return;
      }
    },
    Module.serviceName,
  );

  _log('Starting Flutter app...');
  runApp(new App());

  // runApp(new MaterialApp(
  //   title: 'Counter Parent',
  //   home: new _HomeScreen(key: _homeKey),
  //   theme: new ThemeData(primarySwatch: Colors.orange),
  //   debugShowCheckedModeBanner: false,
  // ));
}
