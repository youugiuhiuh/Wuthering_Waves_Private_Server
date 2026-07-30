# SNI Tester App — Flutter GUI

SNI 测试工具的 Android/Linux 图形界面客户端。

## 架构

Flutter App 通过两种方式调用 Go 引擎 (`../sni_tester/`)：

- **Android**: `dart:ffi` → `libsni_web.so` (CGO 共享库)
- **Linux 桌面**: `Process.start` → `assets/sni_web` (独立二进制)

## 快速开始

```bash
# Android APK (一键)
make android-deploy

# Linux 桌面 (调试用)
make linux-run
```

## Makefile

| 目标 | 说明 |
|------|------|
| `make android-assets` | 编译 `libsni_web.so` (CGO + NDK) |
| `make android-build` | android-assets + Flutter APK |
| `make android-deploy` | 构建 + ADB 安装 |
| `make linux-assets` | 编译 `assets/sni_web` |
| `make linux-run` | assets + Flutter Linux 运行 |
| `make linux-build` | assets + Flutter Linux 构建 |
| `make clean` | 清理 |

## 依赖

- `../sni_tester/` — Go 引擎源码 (Makefile 通过 `go build -C` 引用)
- Flutter SDK、Go、Java 17、Gradle 9.1、Android SDK (由根目录 `mise.toml` 管理)
- Android API 36、Build Tools 36.0.0、NDK 28.2 (用于 CGO 交叉编译)

## 使用 mise 配置环境

```bash
# 仓库根目录
mise install
yes | mise exec -- sdkmanager --licenses
mise exec -- sdkmanager \
  "platforms;android-36" \
  "build-tools;36.0.0" \
  "platform-tools" \
  "ndk;28.2.13676358"

# 构建 Android FFI 资源和 APK
cd sni_tester_app
mise exec -- sh -c 'make android-assets NDK_CC="$ANDROID_SDK_ROOT/ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android21-clang"'
mise exec -- flutter build apk --release

# 验证
mise exec -- flutter test
cd android && mise exec -- gradle :app:compileDebugKotlin
```

`make android-build` 保留用于已配置 Makefile 本机路径的环境；新环境推荐使用以上 mise 命令。

## 项目结构

```
sni_tester_app/
├── lib/
│   ├── data/models/          # Stats, ProgressEvent, StartParams
│   ├── data/services/        # ApiClient (HTTP+SSE), NativeBridge (FFI)
│   │   └── native_bridge.dart  # dart:ffi → libsni_web.so
│   ├── ui/features/home/
│   │   ├── view_models/      # HomeViewModel (ChangeNotifier)
│   │   └── views/            # HomeScreen + 8 widget
│   └── main.dart
├── android/
├── linux/
├── pubspec.yaml
└── Makefile
```
