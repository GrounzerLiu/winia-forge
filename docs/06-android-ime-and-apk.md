# 06 · Android IME and APK build notes

> Summary: GameActivity is the only ready-made path with a full Android InputConnection. cargo-apk2
> is useful for a quick NativeActivity APK and for compiling the .so, but it cannot exercise the
> GameTextInput IME path. The release APK must be assembled by Gradle with a GameActivity
> MainActivity. This document records the working configuration and the build failures encountered
> while bringing up Winia on Android.

## 1. Input method support matrix

| Activity path | Soft keyboard | InputConnection | Preedit / candidates | Commit / delete | Suitable for TextField |
| --- | --- | --- | --- | --- | --- |
| NativeActivity | Usually appears | No | No | Key events only | No |
| GameActivity + GameTextInput | Yes | Yes | Yes | Yes | Yes |
| Custom Activity + custom InputConnection | Yes | Yes, if implemented | Yes, if implemented | Yes, if implemented | Possible, but expensive |

NativeActivity can still produce ASCII input through key events on some keyboards. It does not
provide a Java InputConnection, so Chinese pinyin candidates, composition ranges, IME delete
requests, and surrounding-text updates do not form a reliable text input path.

The current Winia Android path is therefore:

~~~toml
winia = { default-features = false, features = ["android-game-activity"] }
~~~

android-native-activity remains useful for graphics-only prototypes, not for text fields.

"Full" here means the practical text path: preedit, commit, delete-surrounding, selection, and
candidate selection. The current winit fork still leaves TextAction unhandled and
set_ime_cursor_area as a no-op; those are separate follow-ups.

## 2. Published Winia winit fork

The Android work lives in two crates:

| Package | Library name | Purpose |
| --- | --- | --- |
| winia-winit | winit | The main fork; existing use winit::... code is unchanged |
| winia-winit-android | winit_android | Android backend with GameTextInput to WindowEvent::Ime |

The main package keeps the library name winit; only the Cargo package name changes. A project
depending on the published fork writes:

~~~toml
winia-winit = "0.31.0-beta.4"
~~~

and still imports:

~~~rust
use winit::...
~~~

The Android backend is selected by android-game-activity and is pulled in by the main package's
target dependency. Applications do not add winia-winit-android directly unless they are
building a custom Android integration.

## 3. Lifecycle behavior that must be handled

There are two different Android lifecycle cases.

### 3.1 Home and return

The Activity usually survives. Android destroys the native window and sends TerminateWindow;
winit reports it as a surface/backend lifecycle event. The renderer must:

1. Drop the backend surface in destroy_surfaces.
2. Recreate it on InitWindow in can_create_surfaces.
3. Request a redraw after recreation.

If the renderer is not recreated, the window is alive but no frames are submitted, which looks
like a black screen.

### 3.2 Split-screen

ColorOS and several other Android 12+ implementations can destroy and relaunch the Activity in the
same process when entering split-screen. The sequence observed on a device was:

~~~text
Pause
TerminateWindow
Destroy
Resume
InitWindow
~~~

The Activity's onDestroy waits for the Rust entry thread to return. If winit ignores
MainEvent::Destroy, the process blocks and eventually shows "Winia Game IME is not responding".
The fix is to exit the native event loop on Destroy and reset the event-loop creation flag so the
relaunched Activity can create a new loop in the same process.

For renderers, both Vulkan and GL need real recreate_surface implementations. GL must receive the
active event loop because it needs to create a new EGL surface/context.

## 4. Build architecture

Use two separate responsibilities:

~~~text
Rust: cargo-apk2 or cargo-ndk
  -> compile lib<app>.so for arm64-v8a (and other configured ABIs)
  -> copy to android/app/src/main/jniLibs/<abi>/
Gradle:
  -> compile MainActivity.java against androidx.games
  -> merge the native libraries
  -> assemble and sign the APK
~~~

cargo-apk2 alone generates a NativeActivity APK when the manifest uses
android.app.NativeActivity and has_code = false. That is useful to validate rendering and basic
input, but it is not an IME test. The final application must be assembled by Gradle with a
GameActivity subclass.

### 4.1 Recommended source workflow

1. Build the native library with cargo-apk2 (or cargo-ndk).
2. Copy the output to android/app/src/main/jniLibs/arm64-v8a/.
3. Run ./gradlew assembleDebug.
4. Install android/app/build/outputs/apk/debug/app-debug.apk.

Do not treat cargo-apk2's generated APK as the release candidate when the app needs text input.

## 5. Toolchain baseline

The following combination was verified on a Windows host and an Android 15/16 device:

| Component | Working version |
| --- | --- |
| Gradle | 9.5.0 |
| Android Gradle Plugin | 9.3.2 |
| JDK | 21 |
| Kotlin (transitive) | 1.8.22 |
| compileSdk / targetSdk | 36 / 36 |
| minSdk | 26 |
| NDK | 27.0.12077973 |
| LLVM / libclang | 18.1.8 |
| cargo-apk2 | 1.5.0 |
| androidx.games | 4.4.0 |
| androidx.appcompat | 1.7.0 |
| androidx.core | 1.15.0 |

The AGP 9.3.2/Gradle 9.5.0 combination was verified with a clean GRADLE_USER_HOME. An existing
Gradle cache that has recorded failed AGP resolutions can keep reporting missing tools 32.3.2
artifacts even when the repository serves them. Clear the failed metadata or use a fresh Gradle
user home in that case; a local Maven proxy is not a normal project requirement.

androidx.games:games-activity is pinned to 4.4.0 because android-activity 0.6.1 documents
support for that version. 4.4.2 was not A/B verified for the InputConnection path in this workspace;
do not bump it without repeating the real IME test.

minSdk 26 is also the Skia Android baseline. targetSdk must not be lowered just to avoid build
errors; use the current device target (36 in this setup) and adjust AGP instead.

## 6. Native library build

### 6.1 Environment

~~~bash
export ANDROID_NDK_ROOT=C:/Users/<user>/AppData/Local/Android/Sdk/ndk/27.0.12077973
export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"
export ANDROID_NDK="$ANDROID_NDK_ROOT"
export ANDROID_HOME=C:/Users/<user>/AppData/Local/Android/Sdk
export LIBCLANG_PATH=C:/PROGRA~1/LLVM/bin
export BINDGEN_EXTRA_CLANG_ARGS="-resource-dir C:/PROGRA~1/LLVM/lib/clang/18"
export RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
~~~

Build:

~~~bash
cargo apk2 build
~~~

The output library is under target/debug/apk/lib/arm64-v8a/.

### 6.2 libclang and bindgen

skia-bindings uses bindgen. A newer libclang can fail while parsing the Android NDK's libc++
headers with errors such as:

~~~text
error: reference to unresolved using declaration
~~~

The working setup uses LLVM/libclang 18.1.8. Two details matter on Windows:

- Point LIBCLANG_PATH at the short path C:/PROGRA~1/LLVM/bin; spaces in Program Files make the
  bindgen argument parsing fragile.
- When libclang cannot find its built-in headers (for example stddef.h), add
  -resource-dir C:/PROGRA~1/LLVM/lib/clang/18 through BINDGEN_EXTRA_CLANG_ARGS.

The __ANDROID_API__ macro redefined warning from the NDK headers is noisy but not fatal.

### 6.3 16 KB page alignment

Android 15+ checks native libraries for 16 KB page support. cargo-apk2 may set its own
CARGO_ENCODED_RUSTFLAGS, so a rustflags entry in .cargo/config.toml can be ignored. Pass the
flag explicitly in the environment:

~~~bash
export RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
cargo apk2 build
~~~

Verify:

~~~bash
"$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-readelf.exe"   -l target/debug/apk/lib/arm64-v8a/lib<app>.so | grep LOAD
~~~

Each LOAD segment must show alignment 0x4000 (16 KB), not 0x1000 (4 KB).

### 6.4 Build time

- A clean build that has to compile skia-bindings, vulkano, and the winit Android backend can
  take many minutes (about 9 minutes in this workspace).
- Incremental builds after a small Rust change took about 20-30 seconds.
- Avoid cargo clean as a routine troubleshooting step. It turns every iteration into a full
  Skia rebuild.
- Cargo holds a lock on target/; do not run two cargo builds at once.

### 6.5 Cargo network workaround

On this Windows host, cargo occasionally failed to refresh the registry with a schannel
certificate-revocation error. When the required crates are already cached, a build can be forced
offline:

~~~bash
export CARGO_NET_OFFLINE=true
export CARGO_HTTP_CHECK_REVOKE=false
~~~

This is a workaround for a local network/CRL problem, not a normal project requirement.

## 7. Gradle configuration

Use Kotlin DSL and a version catalog. The Groovy + changing-AGP combinations tried first produced
hard-to-localize repository and dependency resolution failures.

### 7.1 settings.gradle.kts

Keep repositories in settings only; do not also add allprojects { repositories { ... } } in the
root build file. Mixing them produced:

~~~text
Build was configured to prefer settings repositories over project repositories
~~~

~~~kotlin
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "my-app"
include(":app")
~~~

### 7.2 Root build.gradle.kts

~~~kotlin
plugins {
    id("com.android.application") version "9.3.2" apply false
}
~~~

If Gradle reports missing AGP artifacts, verify that the exact AGP version is available in the
configured repositories, clear the failed Gradle metadata cache, and retry. Do not silently downgrade
AGP just to make a stale cache disappear.

AGP 9.3 requires Gradle 9.5.0, JDK 17 or newer, and Build Tools 36.0.0. The baseline in this
document uses that combination and was built and launched on a device. AGP 9 is still a major
migration: if a project uses the old variant APIs or custom Gradle plugins, those must be migrated
to the new DSL/androidComponents APIs before the upgrade.

### 7.3 app/build.gradle.kts

~~~kotlin
plugins {
    id("com.android.application")
}

android {
    namespace = "com.example.myapp"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.example.myapp"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
    }
}

configurations.all {
    resolutionStrategy {
        force("org.jetbrains.kotlin:kotlin-stdlib:1.8.22")
    }
    exclude(group = "org.jetbrains.kotlin", module = "kotlin-stdlib-jdk7")
    exclude(group = "org.jetbrains.kotlin", module = "kotlin-stdlib-jdk8")
}

dependencies {
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.core:core:1.15.0")
}
~~~

The androidx.games:games-activity POM used here does not declare its AndroidX runtime
dependencies. appcompat and core must be declared explicitly; otherwise the Activity compiles but
fails at runtime with missing AppCompat or core classes.

The Kotlin exclusions are needed because the AndroidX graph can contain both Kotlin 1.8.22 stdlib
and old kotlin-stdlib-jdk7/jdk8 1.6.21 artifacts. Without them, checkDebugDuplicateClasses fails
with duplicate kotlin.* classes.

### 7.4 AndroidManifest.xml

~~~xml
<application
    android:label="My App"
    android:theme="@style/Theme.AppCompat.NoActionBar">
    <activity
        android:name=".MainActivity"
        android:exported="true"
        android:configChanges="orientation|keyboardHidden|screenSize|smallestScreenSize|screenLayout|keyboard|navigation"
        android:windowSoftInputMode="adjustResize">
        <meta-data
            android:name="android.app.lib_name"
            android:value="my_app" />
        <intent-filter>
            <action android:name="android.intent.action.MAIN" />
            <category android:name="android.intent.category.LAUNCHER" />
        </intent-filter>
    </activity>
</application>
~~~

configChanges reduces unnecessary relaunches, but it is not a substitute for handling
destroy_surfaces / can_create_surfaces and MainEvent::Destroy. Split-screen can still relaunch the
Activity.

### 7.5 MainActivity.java

~~~java
package com.example.myapp;

import com.google.androidgamesdk.GameActivity;

public class MainActivity extends GameActivity {
    static {
        System.loadLibrary("my_app");
    }
}
~~~

The manifest android.app.lib_name and System.loadLibrary should name the same library. The
library must be present in jniLibs/<abi>/ before Gradle packages the APK.

## 8. Native library packaging checklist

A missing .so does not always fail the Gradle build; it can produce an APK that installs and then
crashes or renders black. Check the actual archive:

~~~bash
mkdir -p android/app/src/main/jniLibs/arm64-v8a
cp target/debug/apk/lib/arm64-v8a/libmy_app.so android/app/src/main/jniLibs/arm64-v8a/
cd android
./gradlew assembleDebug
unzip -l app/build/outputs/apk/debug/app-debug.apk | grep libmy_app.so
~~~

If Gradle prints mergeDebugNativeLibs NO-SOURCE, the library was not under the default
src/main/jniLibs path or the directory did not exist when it was copied.

A successful adb install does not prove that the process can load the library. Check:

~~~bash
adb logcat | grep -E "dlopen|UnsatisfiedLinkError|GameActivity"
~~~

## 9. Device verification

### 9.1 Confirm the Activity and InputConnection

~~~bash
adb shell dumpsys activity activities | grep topResumedActivity
adb shell dumpsys input_method | grep -E "mServedInputConnection|mInputShown"
~~~

The served connection should be a com.google.androidgamesdk.gametextinput.InputConnection and the
served view should be GameActivity$InputEnabledSurfaceView. If the connection is missing, the
keyboard may still appear but the app will not receive text events.

### 9.2 Use ADB Keyboard for repeatable IME tests

The OPPO keyboard's on-screen coordinates are awkward to drive from adb, and tapping the wrong
key can open the keyboard toolbox instead of inserting a character. ADB Keyboard sends real
InputConnection edits and is much more reliable for automated verification:

~~~bash
adb shell ime enable com.android.adbkeyboard/.AdbIME
adb shell ime set com.android.adbkeyboard/.AdbIME
adb shell input tap <field-x> <field-y>
adb shell input text hello
~~~

Restore the normal IME afterwards:

~~~bash
adb shell ime set com.oplus.keyboard/.service.ImeService
~~~

### 9.3 Lifecycle tests

Run both:

1. Start the app, go home, reopen it from the launcher.
2. Start the app, enter split-screen with another app, then leave split-screen.

Watch for:

~~~text
Winia Game IME is not responding
ANR in rust.winia_ime_demo_game
~~~

An ANR during split-screen means the Activity was destroyed while the Rust event loop was still
waiting. Check the winit MainEvent::Destroy handling and the event-loop creation flag before
blaming the renderer.

## 10. Troubleshooting table

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| Keyboard appears, no characters | NativeActivity or missing InputConnection | Use GameActivity; verify mServedInputConnection in dumpsys input_method |
| Black screen after home/reopen | Renderer surface was dropped but not recreated | Implement destroy_surfaces / can_create_surfaces; Vulkan/GL recreate_surface |
| Black screen and ANR after split-screen | Activity was destroyed; winit ignored MainEvent::Destroy | Exit native loop on Destroy; reset event-loop flag; allow Activity relaunch |
| mergeDebugNativeLibs NO-SOURCE | .so not under src/main/jniLibs/<abi> | Create the directory before copying; rebuild; inspect APK with unzip -l |
| Install succeeds, process crashes on launch | Library missing or wrong lib name | Check android.app.lib_name, System.loadLibrary, and APK lib/ contents |
| 16 KB alignment warning | ELF LOAD alignment is 0x1000 | Add -C link-arg=-Wl,-z,max-page-size=16384 through RUSTFLAGS; verify 0x4000 |
| reference to unresolved using declaration | libclang cannot parse NDK libc++ | Use LLVM 18; set BINDGEN_EXTRA_CLANG_ARGS resource dir |
| Gradle cannot resolve AGP artifacts | Repository metadata/cache failure | Check the AGP version availability, clear failed cache, retry; use KTS |
| Duplicate Kotlin classes | stdlib and jdk7/jdk8 versions mixed | Force stdlib 1.8.22; exclude jdk7/jdk8 |
| Repository mode error | Repositories declared in both settings and project | Keep them in settings.gradle.kts only |
| Full rebuild takes many minutes | cargo clean, or Skia is compiling from source | Avoid unnecessary cleans; keep incremental target; allow the first build time |

## 11. Do not

- Do not use cargo-apk2's generated NativeActivity APK to test IME.
- Do not set targetSdk below the device requirement just to avoid an AGP warning.
- Do not put a localhost or machine-specific Maven mirror in the committed project.
- Do not rely on on-screen keyboard coordinates in automated tests; use ADB Keyboard.
- Do not publish the main winia-winit package before winia-winit-android; crates.io
  requires the Android dependency to exist first.
- Do not assume a successful install means the native library loaded; inspect the APK and logcat.

## 12. Reference build sequence

~~~bash
# 1. Native library
export ANDROID_NDK_ROOT=.../ndk/27.0.12077973
export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"
export ANDROID_NDK="$ANDROID_NDK_ROOT"
export LIBCLANG_PATH=C:/PROGRA~1/LLVM/bin
export BINDGEN_EXTRA_CLANG_ARGS="-resource-dir C:/PROGRA~1/LLVM/lib/clang/18"
export RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"

cargo apk2 build

# 2. Package the GameActivity APK
mkdir -p android/app/src/main/jniLibs/arm64-v8a
cp target/debug/apk/lib/arm64-v8a/libmy_app.so \
  android/app/src/main/jniLibs/arm64-v8a/
cd android
./gradlew assembleDebug

# 3. Install and launch
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n com.example.myapp/.MainActivity
~~~

The exact commands vary with the project name and ABI list; the important invariant is that the
GameActivity APK is assembled by Gradle after the Rust .so has been copied into jniLibs.
