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
        versionCode = 1
        versionName = "0.3.0"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
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
    kotlinOptions {
        jvmTarget = "17"
    }
    // libflamix_android.so is placed in src/main/jniLibs/<abi>/ by cargo-ndk before assembling.
}
