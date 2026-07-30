SNI_DIR := ../sni_tester
JAVA_HOME := /home/fe/.local/share/mise/installs/java/21.0.2
FLUTTER := /home/fe/.local/share/mise/installs/flutter/latest/bin/flutter
NDK_CC := /home/fe/Android/Sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android21-clang
JNILIBS_DIR := android/app/src/main/jniLibs/arm64-v8a

.PHONY: android-assets android-build android-deploy linux-assets linux-build linux-run clean

android-assets:
	mkdir -p $(JNILIBS_DIR) assets
	CGO_ENABLED=1 GOOS=android GOARCH=arm64 CC=$(NDK_CC) \
		go build -C go_engine -buildmode=c-shared \
		-o $(CURDIR)/$(JNILIBS_DIR)/libsni_web.so .
	touch assets/sni_web

android-build: android-assets
	JAVA_HOME=$(JAVA_HOME) $(FLUTTER) build apk --release

android-deploy: android-build
	@echo "Deploying APK..."
	adb install -r build/app/outputs/flutter-apk/app-release.apk

linux-assets:
	mkdir -p assets
	go build -C $(SNI_DIR) -o $(CURDIR)/assets/sni_web ./cmd/sni_web/

linux-build: linux-assets
	$(FLUTTER) build linux --release

linux-run: linux-assets
	$(FLUTTER) run -d linux

flutter-linux-run: linux-assets
	$(FLUTTER) run -d linux

clean:
	rm -rf build .dart_tool android/app/src/main/jniLibs assets/sni_web
