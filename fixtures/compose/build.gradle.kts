// AGP compiles Kotlin itself (built-in Kotlin), so there is no kotlin-android plugin. The Compose
// compiler plugin depends on the Kotlin Gradle plugin of its own version, which makes the `kotlin`
// version in the catalog the Kotlin compiler for the whole build.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.compose) apply false
}
