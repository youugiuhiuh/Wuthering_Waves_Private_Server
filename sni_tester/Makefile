BINARY=sni_web
OUTPUT_DIR=sni_output

.PHONY: phone-deploy phone-pull clean flutter-build flutter-deploy

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

phone-pull:
	mkdir -p ../rust/aegis/src/resources/sni/
	adb pull /data/local/tmp/$(OUTPUT_DIR)/. ../rust/aegis/src/resources/sni/
	@echo "Results pulled. Run 'cargo build' in rust/aegis to embed."

flutter-build:
	cd flutter_app && flutter build apk --release

flutter-deploy: flutter-build
	@echo "Deploying APK..."
	adb install flutter_app/build/app/outputs/flutter-apk/app-release.apk

clean:
	rm -f $(BINARY)
	rm -rf $(OUTPUT_DIR) badger_db
