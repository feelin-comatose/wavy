plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// номер сборки передаёт GitHub Actions (-PwavyBuild=N) — по нему работает автообновление
val wavyBuild = (findProperty("wavyBuild") as String?)?.toIntOrNull() ?: 1

android {
    namespace = "app.wavy.android"
    compileSdk = 35

    defaultConfig {
        applicationId = "app.wavy.android"
        minSdk = 26
        targetSdk = 35
        versionCode = wavyBuild
        versionName = "0.1.$wavyBuild"
        buildConfigField("int", "WAVY_BUILD", "$wavyBuild")
    }

    // Подпись релиза постоянным ключом (release.p12 в репо зашифрован паролем из секрета CI
    // WAVY_KEYSTORE_PASSWORD) — иначе новая версия не встанет поверх старой.
    val keystorePassword: String? = System.getenv("WAVY_KEYSTORE_PASSWORD")?.takeIf { it.isNotBlank() }
    signingConfigs {
        create("release") {
            if (keystorePassword != null) {
                storeFile = rootProject.file("release.p12")
                storeType = "PKCS12"
                storePassword = keystorePassword
                keyAlias = "wavy"
                keyPassword = keystorePassword
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName(if (keystorePassword != null) "release" else "debug")
        }
    }

    compileOptions {
        isCoreLibraryDesugaringEnabled = true // нужно NewPipeExtractor
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    testOptions {
        unitTests.isIncludeAndroidResources = true
        unitTests.all {
            it.systemProperty("live", project.findProperty("live") ?: "false")
            it.systemProperty("roborazzi.test.record", "true")
            it.testLogging.showStandardStreams = true
        }
    }
    packaging { resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}", "META-INF/DEPENDENCIES") }
}

dependencies {
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs_nio:2.1.3")

    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-service:2.8.7")
    implementation("androidx.lifecycle:lifecycle-process:2.8.7")
    implementation("androidx.savedstate:savedstate-ktx:1.2.1")
    implementation("androidx.core:core-ktx:1.15.0")

    val media3 = "1.5.1"
    implementation("androidx.media3:media3-exoplayer:$media3")
    implementation("androidx.media3:media3-session:$media3")
    implementation("androidx.media3:media3-ui:$media3")
    implementation("androidx.media3:media3-datasource-okhttp:$media3")
    implementation("androidx.media3:media3-database:$media3")

    implementation("com.github.TeamNewPipe:NewPipeExtractor:v0.27.1")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("io.coil-kt.coil3:coil-compose:3.0.4")
    implementation("io.coil-kt.coil3:coil-network-okhttp:3.0.4")

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.14.1")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("io.github.takahirom.roborazzi:roborazzi:1.39.0")
    testImplementation("io.github.takahirom.roborazzi:roborazzi-compose:1.39.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
