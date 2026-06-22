BINARY=sni_web
OUTPUT_DIR=sni_output

.PHONY: phone-deploy phone-pull clean flutter-assets flutter-build flutter-deploy linux-build linux-run

phone-deploy: $(BINARY)
	adb push $(BINARY) /data/local/tmp/
	adb push GeoLite2-Country.mmdb /data/local/tmp/ 2>/dev/null || true
	adb push GeoLite2-ASN.mmdb /data/local/tmp/ 2>/dev/null || true
	adb shell "mkdir -p /data/local/tmp/$(OUTPUT_DIR)"
	adb shell "cd /data/local/tmp && SNI_OUTPUT_DIR=/data/local/tmp/$(OUTPUT_DIR) chmod +x $(BINARY) && ./$(BINARY)"
	adb forward tcp:18080 tcp:18080
	@echo "Open http://localhost:18080 in your browser (or use Flutter app)"

$(BINARY):
	GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -o $(BINARY) ./cmd/sni_web/

# Bundle Go binary into Flutter assets
flutter-assets:
	GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -o flutter_app/assets/sni_web ./cmd/sni_web/

phone-pull:
	mkdir -p ../rust/aegis/src/resources/sni/
	adb pull /data/local/tmp/$(OUTPUT_DIR)/. ../rust/aegis/src/resources/sni/
	@echo "Results pulled. Run 'cargo build' in rust/aegis to embed."

flutter-build: flutter-assets
	cd flutter_app && flutter build apk --release

flutter-deploy: flutter-build
	@echo "Deploying APK..."
	adb install flutter_app/build/app/outputs/flutter-apk/app-release.apk

# Linux desktop build with bundled Go binary
linux-build:
	go build -o flutter_app/assets/sni_web ./cmd/sni_web/
	cd flutter_app && flutter build linux --release

linux-run:
	cd flutter_app && flutter run -d linux

# Run Flutter desktop with Go backend auto-built
flutter-linux-run:
	go build -o flutter_app/assets/sni_web ./cmd/sni_web/
	cd flutter_app && flutter run -d linux

clean:
	rm -f $(BINARY)
	rm -f flutter_app/assets/sni_web
	rm -rf $(OUTPUT_DIR) badger_db
