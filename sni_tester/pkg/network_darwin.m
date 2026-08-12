#import <CoreWLAN/CoreWLAN.h>

const char *wifi_interface_name(void) {
    return [[[CWWiFiClient sharedWiFiClient] interface] interfaceName].UTF8String;
}
