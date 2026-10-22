plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "domica.caprust.remote"
    compileSdk = 34

    defaultConfig {
        applicationId = "domica.caprust.remote"
        // API 21 is what the NDK clang wrappers are named for and what
        // covers the devices worth supporting. minSdk has nothing to do
        // with the Rust engine here: this app only speaks JSON over a
        // socket.
        minSdk = 21
        targetSdk = 34
        versionCode = 1
        versionName = "0.9.18"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The only dependency in the whole project, and it is not optional:
    // Kotlin has no runtime without it. org.json ships with the
    // platform, so JSON-RPC still needs nothing added, and there is no
    // networking or lifecycle library to drift against.
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-stdlib:1.9.24")
}

// The engine, as a native binary the app starts.
//
// Android 10 and later do not let an app run a binary out of its own
// data directory. The native library folder is the one place an app
// may run one from: the package manager extracts it there and sets
// the exec bit.
//
// Two architectures: a phone is arm64, an emulator on a laptop is
// x86_64. The app finds the engine in its own native library folder,
// which Android fills for the architecture it is running on, so the
// APK has to carry both.
//
// The ffmpeg goes the same way, into the same folder: the engine runs
// it as a separate program, and a program can only be run from there.
val jniLibs = file("src/main/jniLibs")

val arm64Target = "aarch64-linux-android"
val x64Target = "x86_64-linux-android"

fun engineBinary(target: String) =
    rootProject.file("../target/$target/release/caprust-androidd")

fun ffmpegBinary(abi: String) = rootProject.file("../android/ffmpeg/$abi/ffmpeg")

val copyEngineArm64 by tasks.registering(Copy::class) {
    from(engineBinary(arm64Target)) {
        into(File(jniLibs, "arm64-v8a"))
        rename { "libcaprust-androidd.so" }
    }
    // The binary is not there when the app is built without the
    // engine (a plain `gradle assembleDebug` on a laptop). The app
    // then says the engine is missing, instead of the build failing.
    onlyIf { engineBinary(arm64Target).exists() }
}

val copyEngineX64 by tasks.registering(Copy::class) {
    from(engineBinary(x64Target)) {
        into(File(jniLibs, "x86_64"))
        rename { "libcaprust-androidd.so" }
    }
    onlyIf { engineBinary(x64Target).exists() }
}

val copyFfmpegArm64 by tasks.registering(Copy::class) {
    from(ffmpegBinary("arm64-v8a")) {
        into(File(jniLibs, "arm64-v8a"))
        rename { "libffmpeg.so" }
    }
    onlyIf { ffmpegBinary("arm64-v8a").exists() }
}

val copyFfmpegX64 by tasks.registering(Copy::class) {
    from(ffmpegBinary("x86_64")) {
        into(File(jniLibs, "x86_64"))
        rename { "libffmpeg.so" }
    }
    onlyIf { ffmpegBinary("x86_64").exists() }
}

tasks.named("preBuild") {
    dependsOn(copyEngineArm64, copyEngineX64, copyFfmpegArm64, copyFfmpegX64)
}
