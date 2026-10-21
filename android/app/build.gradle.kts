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
// The binary is built for aarch64-linux-android (the CI does it) and
// copied in here. The ffmpeg goes the same way, into the same folder:
// the engine runs it as a separate program, and a program can only be
// run from there.
val jniLibs = file("src/main/jniLibs")
val arm64 = File(jniLibs, "arm64-v8a")

val engineBinary =
    rootProject.file("../target/aarch64-linux-android/release/caprust-androidd")

val copyEngine by tasks.registering(Copy::class) {
    from(engineBinary)
    into(arm64)
    rename { "libcaprust-androidd.so" }
    // The binary is not there when the app is built without the
    // engine (a plain `gradle assembleDebug` on a laptop). The app
    // then says the engine is missing, instead of the build failing.
    onlyIf { engineBinary.exists() }
}

val copyFfmpeg by tasks.registering(Copy::class) {
    from(rootProject.file("../android/ffmpeg/arm64-v8a/ffmpeg"))
    into(arm64)
    rename { "libffmpeg.so" }
    onlyIf { rootProject.file("../android/ffmpeg/arm64-v8a/ffmpeg").exists() }
}

tasks.named("preBuild") {
    dependsOn(copyEngine, copyFfmpeg)
}
