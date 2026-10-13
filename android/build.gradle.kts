plugins {
    id("com.android.application") version "8.5.2" apply false
    // Without this the Kotlin plugin is never applied, .kt files under
    // src/main/java are silently skipped, and `assembleDebug` still
    // exits 0 -- producing an APK with an empty classes.dex that
    // crashes on launch with ClassNotFoundException. The first build
    // did exactly that: green CI, 1804-byte dex.
    id("org.jetbrains.kotlin.android") version "1.9.24" apply false
}