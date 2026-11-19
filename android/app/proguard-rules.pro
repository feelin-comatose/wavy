# NewPipeExtractor и Rhino (расшифровка подписей YouTube) работают через рефлексию
-keep class org.schabi.newpipe.extractor.** { *; }
-keep class org.mozilla.javascript.** { *; }
-keep class org.mozilla.classfile.ClassFileWriter
-dontwarn org.mozilla.javascript.**
-dontwarn javax.script.**
-dontwarn java.beans.**
-dontwarn jdk.dynalink.**
-dontwarn org.jsoup.**
-dontwarn com.google.re2j.**
-dontwarn org.slf4j.**
# модели библиотеки сериализуются kotlinx.serialization
-keepclassmembers class app.wavy.android.data.** { *; }
# protobuf-lite (запросы NewPipe к YouTube) читает поля сообщений через рефлексию — имена менять нельзя
-keep class * extends com.google.protobuf.GeneratedMessageLite { *; }
-dontwarn com.google.protobuf.**
# Без переименования классов: R8 только выкидывает неиспользуемый код. Библиотеки с рефлексией
# (NewPipe, Rhino, protobuf, kotlinx.serialization) так гарантированно находят всё по именам.
-dontobfuscate
-keep class app.wavy.android.data.** { *; }
