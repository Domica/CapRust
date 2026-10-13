plugins {
    id("com.android.application")
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
        versionName = "0.9.16"
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

    // No dependencies beyond the platform. org.json ships with Android,
    // so JSON-RPC needs nothing added, and a dependency-free build does
    // not drift when the runner's cached Gradle and AGP versions move.
}