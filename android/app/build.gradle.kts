import java.io.File
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

fun signingFromEnvOrFile(): Map<String, String>? {
    val keystore = System.getenv("KRAPY_KEYSTORE")?.takeIf { it.isNotBlank() }
    val storePass = System.getenv("KRAPY_KEYSTORE_PASSWORD")?.takeIf { it.isNotBlank() }
    val alias = System.getenv("KRAPY_KEY_ALIAS")?.takeIf { it.isNotBlank() }
    val keyPass = System.getenv("KRAPY_KEY_PASSWORD")?.takeIf { it.isNotBlank() }
    if (keystore != null && storePass != null && alias != null && keyPass != null) {
        return mapOf(
            "storeFile" to keystore,
            "storePassword" to storePass,
            "keyAlias" to alias,
            "keyPassword" to keyPass,
        )
    }
    val propsFile = rootProject.file("keystore.properties")
    if (!propsFile.isFile) {
        return null
    }
    val props = Properties()
    propsFile.inputStream().use { props.load(it) }
    val storeFile = props.getProperty("storeFile")?.takeIf { it.isNotBlank() }
    val storePassword = props.getProperty("storePassword")?.takeIf { it.isNotBlank() }
    val keyAlias = props.getProperty("keyAlias")?.takeIf { it.isNotBlank() }
    val keyPassword = props.getProperty("keyPassword")?.takeIf { it.isNotBlank() }
    if (storeFile == null || storePassword == null || keyAlias == null || keyPassword == null) {
        return null
    }
    return mapOf(
        "storeFile" to storeFile,
        "storePassword" to storePassword,
        "keyAlias" to keyAlias,
        "keyPassword" to keyPassword,
    )
}

val releaseSigning = signingFromEnvOrFile()

android {
    namespace = "com.waghitech.krapy"
    compileSdk = 36
    buildToolsVersion = "36.0.0"

    defaultConfig {
        applicationId = "com.waghitech.krapy"
        minSdk = 29
        targetSdk = 36
        versionCode = 6
        versionName = "0.1.5"
    }

    if (releaseSigning != null) {
        signingConfigs {
            create("release") {
                val path = releaseSigning["storeFile"]!!
                storeFile =
                    if (File(path).isAbsolute) {
                        file(path)
                    } else {
                        rootProject.file(path)
                    }
                storePassword = releaseSigning["storePassword"]
                keyAlias = releaseSigning["keyAlias"]
                keyPassword = releaseSigning["keyPassword"]
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            if (releaseSigning != null) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    lint {
        abortOnError = false
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
        unitTests.isIncludeAndroidResources = true
    }

    @Suppress("DEPRECATION")
    applicationVariants.configureEach {
        if (buildType.name != "release") {
            return@configureEach
        }
        val vn = versionName ?: defaultConfig.versionName ?: "0.1.0"
        outputs.configureEach {
            (this as com.android.build.gradle.api.ApkVariantOutput).outputFileName =
                "krapy-$vn.apk"
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

dependencies {
    implementation("com.google.zxing:core:3.5.3")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.14.1")
}