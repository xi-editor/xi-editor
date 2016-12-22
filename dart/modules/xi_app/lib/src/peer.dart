import 'package:apps.xi.services/xi.fidl.dart';
import 'package:lib.fidl.dart/bindings.dart';
import 'package:lib.fidl.dart/core.dart' as core;
import 'dart:typed_data';
import 'dart:convert';

/// A callback for string data sent by xi-core.
typedef void StringCallback(String data);

/// I connection to xi-core.
class XiPeer {
  /// The callback triggered when data is sent.
  StringCallback onRead;

  final JsonProxy _jsonProxy = new JsonProxy();
  core.SocketReader _reader = new core.SocketReader();
  Uint8List _buf = new Uint8List(4096);
  List<int> _fragment = new List<int>();
  static const Utf8Encoder _utf8Encoder = const Utf8Encoder();
  static const Utf8Decoder _utf8Decoder = const Utf8Decoder();
  static const int _newlineChar = 0x0a;

  /// Bind the the xi-core service.
  void bind(InterfaceHandle handle) {
    _jsonProxy.ctrl.bind(handle);
    final core.SocketPair pair = new core.SocketPair();
    _jsonProxy.connectSocket(pair.socket0);
    _reader.bind(pair.passSocket1());
    _reader.onReadable = _handleRead;
  }

  /// Send data to xi-core.
  void send(String string) {
    final List<int> utf8 = _utf8Encoder.convert(string);
    final Uint8List bytes = new Uint8List.fromList(utf8);
    _reader.socket.write(bytes.buffer.asByteData());
  }

  void _handleRead() {
    final core.SocketReadResult readResult =
        _reader.socket.read(_buf.buffer.asByteData());
    if (readResult.status == core.NO_ERROR) {
      int start = 0;
      int length = readResult.bytesRead;
      if (onRead != null) {
        for (int i = 0; i < length; i++) {
          if (_buf[i] == _newlineChar) {
            _fragment.addAll(_buf.getRange(start, i + 1));
            start = i + 1;
            String string = _utf8Decoder.convert(_fragment);
            onRead(string);
            _fragment.clear();
          }
        }
      }
      if (start < length) {
        _fragment.addAll(_buf.getRange(start, length));
      }
    }
  }
}
