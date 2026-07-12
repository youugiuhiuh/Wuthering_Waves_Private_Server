import 'dart:ffi';
import 'package:ffi/ffi.dart';

typedef StartServerNative = Int32 Function(Pointer<Utf8>, Pointer<Utf8>);
typedef StartServerDart = int Function(Pointer<Utf8>, Pointer<Utf8>);

typedef StopServerNative = Int32 Function();
typedef StopServerDart = int Function();

class NativeBridge {
  late final DynamicLibrary _lib;
  late final StartServerDart _startServer;
  late final StopServerDart _stopServer;

  NativeBridge() {
    _lib = DynamicLibrary.open('libsni_web.so');
    _startServer = _lib.lookupFunction<StartServerNative, StartServerDart>('StartServer');
    _stopServer = _lib.lookupFunction<StopServerNative, StopServerDart>('StopServer');
  }

  int startServer(String baseDir, String outputDir) {
    final basePtr = baseDir.toNativeUtf8();
    final outputPtr = outputDir.toNativeUtf8();
    try {
      return _startServer(basePtr, outputPtr);
    } finally {
      calloc.free(basePtr);
      calloc.free(outputPtr);
    }
  }

  int stopServer() => _stopServer();
}
