// Release version comes from CI (tag vX.Y.Z -> VERSION_NAME); versionCode = major*1_000_000 + minor*1_000 + patch.
val appVersionName: String = System.getenv("VERSION_NAME") ?: "0.3.0"
val appVersionCode: Int = System.getenv("VERSION_CODE")?.toIntOrNull() ?: run {
    val p = appVersionName.substringBefore('-').split('.').map { it.toIntOrNull() ?: 0 }
    p.getOrElse(0) { 0 } * 1_000_000 + p.getOrElse(1) { 0 } * 1_000 + p.getOrElse(2) { 0 }
}
// Per-ABI splits only for release builds; debug stays a single APK (app-debug.apk) for the emulator check.
val releaseBuild = gradle.startParameter.taskNames.any { it.contains("release", ignoreCase = true) }
val keystorePath: String? = System.getenv("ANDROID_KEYSTORE_PATH")

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "online.flamix"
    compileSdk = 34

    defaultConfig {
        applicationId = "online.flamix.client"
        minSdk = 26
        targetSdk = 34
        versionCode = appVersionCode
        versionName = appVersionName
        if (!releaseBuild) {
            ndk {
                abiFilters += listOf("arm64-v8a", "x86_64")
            }
        }
    }

    if (releaseBuild) {
        splits {
            abi {
                isEnable = true
                reset()
                include("arm64-v8a", "x86_64")
                isUniversalApk = false
            }
        }
    }

    signingConfigs {
        if (keystorePath != null) {
            create("release") {
                storeFile = file(keystorePath)
                storeType = "PKCS12"
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (keystorePath != null) signingConfig = signingConfigs.getByName("release")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    // libflamix_android.so is placed in src/main/jniLibs/<abi>/ by cargo-ndk before assembling.
}
