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