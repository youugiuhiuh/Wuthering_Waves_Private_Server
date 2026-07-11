import 'dart:io' show Platform;

bool get isDesktop =>
    Platform.isLinux || Platform.isWindows || Platform.isMacOS;

bool get isMobile => Platform.isAndroid || Platform.isIOS;
